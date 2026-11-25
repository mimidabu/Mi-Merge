use crate::cli::Cli;
use crate::discovery::resolve_inputs;
use crate::ffmpeg_bin::MediaTools;
use crate::plan::{format_command_line, render_concat_list, MergeMode, MergePlan};
use crate::probe::probe_clip;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::NamedTempFile;

pub fn run(cli: &Cli) -> Result<()> {
    if cli.output.exists() && !cli.overwrite && !cli.dry_run {
        bail!(
            "output file already exists: {} (use -y to overwrite)",
            cli.output.display()
        );
    }

    let tools = MediaTools::locate(cli.ffmpeg.as_deref(), cli.ffprobe.as_deref())?;
    let input_files = resolve_inputs(&cli.inputs)?;

    let mut clips = Vec::with_capacity(input_files.len());
    for file in &input_files {
        let info = probe_clip(&tools.ffprobe, file)?;
        clips.push(info);
    }

    let plan = MergePlan::build(
        clips,
        cli.resolution,
        cli.fps,
        cli.encoder,
        cli.chapters,
    )?;

    if cli.dry_run {
        print_dry_run(&tools.ffmpeg, &plan, &cli.output, cli.overwrite)?;
        return Ok(());
    }

    execute_plan(&tools.ffmpeg, &plan, &cli.output, cli.overwrite)
}

fn print_dry_run(
    ffmpeg_path: &Path,
    plan: &MergePlan,
    output_path: &Path,
    overwrite: bool,
) -> Result<()> {
    let mode_label = match plan.mode {
        MergeMode::FastCopy => "fast-copy (concat demuxer, -c copy)",
        MergeMode::Transcode => "transcode (filter_complex)",
    };

    println!("Mode:       {mode_label}");
    println!("Target:     {} @ {} fps", plan.target_resolution, plan.target_fps);
    println!("Clips:      {}", plan.clips.len());
    if plan.ffmetadata.is_some() {
        for (i, ch) in plan.chapters.iter().enumerate() {
            println!(
                "  [{:02}] {:>7}ms .. {:>7}ms  {}",
                i + 1,
                ch.start_ms,
                ch.end_ms,
                ch.title
            );
        }
    } else {
        println!("Chapters:   disabled");
    }

    let fake_concat = Path::new("concat_list.txt");
    let fake_meta = Path::new("ffmetadata.txt");
    let concat_opt = if plan.mode == MergeMode::FastCopy {
        Some(fake_concat)
    } else {
        None
    };
    let meta_opt = if plan.ffmetadata.is_some() {
        Some(fake_meta)
    } else {
        None
    };

    let args = plan.build_ffmpeg_args(concat_opt, meta_opt, output_path, overwrite)?;
    println!("\nFFmpeg command:");
    println!("{}", format_command_line(ffmpeg_path, &args));

    Ok(())
}

pub fn execute_plan(
    ffmpeg_path: &Path,
    plan: &MergePlan,
    output_path: &Path,
    overwrite: bool,
) -> Result<()> {
    let _meta_guard: Option<NamedTempFile>;
    let meta_path_buf;
    let meta_opt = if let Some(ref ffmetadata) = plan.ffmetadata {
        let mut meta_file =
            NamedTempFile::new().context("failed to create temporary metadata file")?;
        meta_file
            .write_all(ffmetadata.as_bytes())
            .context("failed to write ffmetadata file")?;
        meta_file.flush()?;
        meta_path_buf = meta_file.path().to_path_buf();
        _meta_guard = Some(meta_file);
        Some(meta_path_buf.as_path())
    } else {
        _meta_guard = None;
        None
    };

    let _concat_guard: Option<NamedTempFile>;
    let concat_path_buf;
    let concat_opt = if plan.mode == MergeMode::FastCopy {
        let mut concat_file =
            NamedTempFile::new().context("failed to create temporary concat list file")?;
        let list_content = render_concat_list(&plan.clips)?;
        concat_file
            .write_all(list_content.as_bytes())
            .context("failed to write concat list file")?;
        concat_file.flush()?;
        concat_path_buf = concat_file.path().to_path_buf();
        _concat_guard = Some(concat_file);
        Some(concat_path_buf.as_path())
    } else {
        _concat_guard = None;
        None
    };

    let args = plan.build_ffmpeg_args(concat_opt, meta_opt, output_path, overwrite)?;

    let status = Command::new(ffmpeg_path)
        .args(&args)
        .status()
        .with_context(|| format!("failed to launch ffmpeg ({})", ffmpeg_path.display()))?;

    if !status.success() {
        bail!("ffmpeg exited with status: {status}");
    }

    Ok(())
}
