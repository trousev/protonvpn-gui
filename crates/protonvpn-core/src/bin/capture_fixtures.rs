//! Captures `protonvpn` output through a real PTY into the parser test corpus.
//!
//! This exists because the first round of fixtures was captured through a pipe, and a TTY can
//! change what the CLI emits — colours, progress rendering, possibly wrapping. Parsers must be
//! written against what the application will actually see, which is a PTY.
//!
//! ```text
//! capture-fixtures --name status_disconnected --cols 120 -- protonvpn status
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use protonvpn_core::pty;

const DEFAULT_OUT_DIR: &str = "crates/protonvpn-core/tests/fixtures";

struct Options {
    name: String,
    cols: u16,
    rows: u16,
    out_dir: PathBuf,
    argv: Vec<String>,
}

fn usage() -> String {
    format!(
        "usage: capture-fixtures --name <fixture> [--cols N] [--rows N] [--out-dir DIR] -- <cmd> [args...]\n\
         \n\
         Captures raw PTY output to <out-dir>/pty/<name>.txt and metadata to\n\
         <out-dir>/pty/<name>.meta.json. Default out-dir: {DEFAULT_OUT_DIR}"
    )
}

fn parse_args() -> Result<Options, String> {
    let mut args = env::args().skip(1);
    let mut name: Option<String> = None;
    let mut cols: u16 = 120;
    let mut rows: u16 = 40;
    let mut out_dir = PathBuf::from(DEFAULT_OUT_DIR);
    let mut argv: Option<Vec<String>> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = Some(args.next().ok_or("--name needs a value")?),
            "--cols" => {
                cols = args
                    .next()
                    .ok_or("--cols needs a value")?
                    .parse()
                    .map_err(|e| format!("bad --cols: {e}"))?
            }
            "--rows" => {
                rows = args
                    .next()
                    .ok_or("--rows needs a value")?
                    .parse()
                    .map_err(|e| format!("bad --rows: {e}"))?
            }
            "--out-dir" => out_dir = PathBuf::from(args.next().ok_or("--out-dir needs a value")?),
            "--" => {
                argv = Some(args.by_ref().collect());
                break;
            }
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unexpected argument: {other}\n\n{}", usage())),
        }
    }

    let argv = argv.ok_or_else(|| format!("missing `-- <cmd>`\n\n{}", usage()))?;
    if argv.is_empty() {
        return Err(format!("empty command after `--`\n\n{}", usage()));
    }

    Ok(Options {
        name: name.ok_or_else(|| format!("missing --name\n\n{}", usage()))?,
        cols,
        rows,
        out_dir,
        argv,
    })
}

fn count_ansi(input: &str) -> usize {
    input.matches('\u{1b}').count()
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn write_meta(path: &Path, opts: &Options, inv: &pty::Invocation) -> std::io::Result<()> {
    let argv_json = opts
        .argv
        .iter()
        .map(|a| format!("\"{}\"", json_escape(a)))
        .collect::<Vec<_>>()
        .join(", ");

    let meta = format!(
        "{{\n  \
         \"name\": \"{}\",\n  \
         \"argv\": [{}],\n  \
         \"command_line\": \"{}\",\n  \
         \"cols\": {},\n  \
         \"rows\": {},\n  \
         \"exit_code\": {},\n  \
         \"duration_ms\": {},\n  \
         \"bytes\": {},\n  \
         \"ansi_sequences\": {},\n  \
         \"had_carriage_return\": {}\n\
         }}\n",
        json_escape(&opts.name),
        argv_json,
        json_escape(&inv.command_line()),
        inv.cols,
        inv.rows,
        inv.exit_code,
        inv.duration.as_millis(),
        inv.output.len(),
        count_ansi(&inv.output),
        inv.output.contains('\r'),
    );

    fs::write(path, meta)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let pty_dir = opts.out_dir.join("pty");
    if let Err(e) = fs::create_dir_all(&pty_dir) {
        eprintln!("cannot create {}: {e}", pty_dir.display());
        return ExitCode::FAILURE;
    }

    eprintln!("$ {}", opts.argv.join(" "));
    let invocation = match pty::run(&opts.argv, opts.cols, opts.rows) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("capture failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let txt_path = pty_dir.join(format!("{}.txt", opts.name));
    let meta_path = pty_dir.join(format!("{}.meta.json", opts.name));

    if let Err(e) = fs::write(&txt_path, &invocation.output) {
        eprintln!("cannot write {}: {e}", txt_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = write_meta(&meta_path, &opts, &invocation) {
        eprintln!("cannot write {}: {e}", meta_path.display());
        return ExitCode::FAILURE;
    }

    println!(
        "{:<28} exit={:<4} {:>5}ms {:>5}B  ansi={:<3} cr={:<5} {}",
        opts.name,
        invocation.exit_code,
        invocation.duration.as_millis(),
        invocation.output.len(),
        count_ansi(&invocation.output),
        invocation.output.contains('\r'),
        txt_path.display(),
    );

    ExitCode::SUCCESS
}
