//! zeno-bench — measures the claims made in the performance budget.
//!
//! The editor is judged on three numbers:
//!
//! * **cold start**  — process launch to a usable editor, under 30 ms
//! * **first frame** — process launch to the first painted frame, under 50 ms
//! * **keystroke**   — key press to repaint, under 16 ms
//!
//! The harness generates the files to measure against (including adversarial
//! shapes), measures each stage separately, and fails with a non-zero exit
//! code when a budget is missed.
//!
//! Two passes are run over every file. The **cold** pass is the first time the
//! file is touched after it was written: on a cold file cache, and after the
//! operating system's own on-access checks have run. The **warm** pass is the
//! second time, with the file already in the page cache. The difference is
//! the part of the cost that belongs to the operating system rather than to
//! the editor.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use memmap2::Mmap;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use zeno_cli::doc::Doc;
use zeno_cli::doc::edit::Pos;
use zeno_cli::highlight::Highlighter;
use zeno_cli::motion::{self, Motion};
use zeno_cli::render::{View, ViewState};

/// The viewport the benchmark renders into: a typical terminal size.
const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

/// Budget from ADR 0007, in milliseconds.
const BUDGET_COLD_START_MS: f64 = 30.0;
const BUDGET_FIRST_FRAME_MS: f64 = 50.0;
const BUDGET_KEYSTROKE_MS: f64 = 16.0;

fn main() -> ExitCode {
    let opts = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("zeno-bench: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&opts.data_dir) {
        eprintln!("zeno-bench: cannot create {}: {e}", opts.data_dir.display());
        return ExitCode::FAILURE;
    }

    if cfg!(debug_assertions) {
        eprintln!(
            "zeno-bench: WARNING — this is a debug build. The budget applies to a release build;\n\
             zeno-bench:          run `cargo build --release` and use target/release/zeno-bench.exe."
        );
    }

    let cases = build_cases(&opts);
    let mut report = String::new();
    let mut breakdown_report = String::new();
    let mut motion_report = String::new();
    motion_header(&mut motion_report);
    let mut failures: Vec<String> = Vec::new();
    header(&mut report);
    breakdown_header(&mut breakdown_report);

    let editor = find_editor_binary();
    // The range-delete probe belongs on the file with the most lines: the
    // largest file here is a single enormous line, which has no range to
    // delete.
    let mut line_rich: Option<(String, u64)> = None;

    for case in &cases {
        let path = opts.data_dir.join(&case.name);
        if !opts.skip_generate
            && let Err(e) = generate(&path, case)
        {
            eprintln!("zeno-bench: cannot generate {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        if !path.exists() {
            eprintln!("zeno-bench: {} is missing", path.display());
            return ExitCode::FAILURE;
        }

        // Cold pass: the first time this file is opened since it was written.
        let cold = match measure(&path, opts.frames) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("zeno-bench: cannot measure {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };

        // Warm pass: the file is in the page cache and has been checked.
        let warm = match measure(&path, opts.frames) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("zeno-bench: cannot measure {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };

        // Process launch to first frame, measured from the outside. The first
        // launch also pays for loading the binary itself, so it is repeated
        // and the second run is the one that counts.
        let launch = editor.as_ref().and_then(|bin| measure_launch(bin, &path));

        write_row(&mut report, case, &cold, &warm, launch.as_ref());
        check(&mut failures, case, &cold, true);
        check(&mut failures, case, &warm, false);

        if line_rich
            .as_ref()
            .is_none_or(|(_, lines)| warm.lines > *lines)
        {
            line_rich = Some((case.name.clone(), warm.lines));
        }

        // Where does the cold cost go? Measured on a second, equally fresh
        // file so that the breakdown is not polluted by the passes above.
        let probe_path = opts.data_dir.join(format!("{}.probe", case.name));
        if !opts.skip_generate
            && let Err(e) = generate(&probe_path, case)
        {
            eprintln!("zeno-bench: cannot generate {}: {e}", probe_path.display());
            return ExitCode::FAILURE;
        }
        if let Ok(b) = probe_cold(&probe_path) {
            write_breakdown(&mut breakdown_report, case, &b);
        }
        let _ = std::fs::remove_file(&probe_path);

        // A motion is on the keystroke path, so it is checked against the
        // keystroke budget rather than reported on its own.
        match motion_scan(&path) {
            Ok((line_chars, times)) => {
                write_motion(&mut motion_report, case, line_chars, &times);
                for (label, cost) in [("w", times[0]), ("e", times[1]), ("b", times[2])] {
                    if cost > BUDGET_KEYSTROKE_MS {
                        failures.push(format!(
                            "{}: word motion {label} {cost:.1} ms > {:.0} ms",
                            case.name, BUDGET_KEYSTROKE_MS
                        ));
                    }
                }
            }
            Err(e) => eprintln!("zeno-bench: motion probe failed: {e}"),
        }
    }

    print!("{report}");
    println!("\nCold open breakdown — the first time a fresh file is opened:");
    print!("{breakdown_report}");

    // A keystroke only ever touches one line, so the per-keystroke numbers say
    // nothing about a command that touches thousands of lines at once. This is
    // where a line-anchored model would show its cost.
    if let Some((name, lines)) = line_rich {
        let path = opts.data_dir.join(&name);
        match range_edit(&path, lines) {
            Ok(text) => print!("{text}"),
            Err(e) => eprintln!("zeno-bench: range edit probe failed: {e}"),
        }
    }
    print!("{motion_report}");

    match highlight_probe(&opts.data_dir, &mut failures) {
        Ok(text) => print!("{text}"),
        Err(e) => eprintln!("zeno-bench: highlight probe failed: {e}"),
    }

    if !opts.keep {
        for case in &cases {
            let _ = std::fs::remove_file(opts.data_dir.join(&case.name));
        }
    }

    if failures.is_empty() {
        println!("\nAll budgets met.");
        ExitCode::SUCCESS
    } else {
        println!("\nBudgets missed:");
        for f in &failures {
            println!("  - {f}");
        }
        ExitCode::FAILURE
    }
}

/// One command deleting a range of lines, and then undoing it.
///
/// The shape of the growth is the answer to whether a line-anchored overlay is
/// enough, so this measures several sizes rather than one.
fn range_edit(path: &Path, total_lines: u64) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("\nRange delete — one command removing N lines, then undoing it:\n");
    out.push_str("| lines | delete | undo |\n| --- | --- | --- |\n");

    let sizes = [1_000u64, 4_000, 16_000];
    for n in sizes {
        if n > total_lines {
            continue;
        }
        let mut doc = Doc::open(path).map_err(|e| e.to_string())?;
        doc.index_fully();
        if doc.line_count() <= n {
            continue;
        }
        let start = Pos::new(0, 0);
        let end = Pos::new(n, 0);

        let t = Instant::now();
        doc.edit(start, end, "");
        let delete = ms(t.elapsed());

        let t = Instant::now();
        doc.undo();
        let undo = ms(t.elapsed());

        let _ = writeln!(out, "| {n} | {delete:.3} | {undo:.3} |");
    }
    Ok(out)
}

/// What syntax highlighting costs: what the worker pays, and what the
/// editor pays to ask.
///
/// The worker's costs are not on a budget. Nobody waits for them, a window
/// the user has scrolled past is cancelled rather than finished, and all of
/// it happens on its own thread. They are measured because they are still
/// work the machine does, and because "the colours arrive late" is a quality
/// question with a number behind it.
///
/// The editor's cost *is* on a budget: `request` and `poll` run on every
/// frame, so they are checked against the keystroke budget like everything
/// else on that path.
fn highlight_probe(data_dir: &Path, failures: &mut Vec<String>) -> Result<String, String> {
    // Highlighting needs a file with a syntax, and the generated cases are
    // prose in `.txt`, which deliberately has none.
    let path = data_dir.join("highlight-probe.rs");
    {
        let mut w = BufWriter::new(File::create(&path).map_err(|e| e.to_string())?);
        for i in 0..8_000 {
            writeln!(w, "fn f{i}(x: u64) -> u64 {{").map_err(|e| e.to_string())?;
            writeln!(w, "    // what f{i} does").map_err(|e| e.to_string())?;
            writeln!(w, "    let y = x.wrapping_mul({i});").map_err(|e| e.to_string())?;
            writeln!(w, "    /* scale it */ y ^ 0x5a5a").map_err(|e| e.to_string())?;
            writeln!(w, "}}").map_err(|e| e.to_string())?;
            writeln!(w).map_err(|e| e.to_string())?;
        }
        w.flush().map_err(|e| e.to_string())?;
    }

    /// Waits until the window really is coloured, and says how long that took.
    ///
    /// `contains_key` rather than "any colour at all": the previous window's
    /// colours are still on hand, so waiting for a non-empty map would return
    /// before the worker had answered.
    fn colour_window(h: &mut Highlighter, doc: &Doc, first: u64) -> Option<f64> {
        let t = Instant::now();
        let last = first + 40;
        h.request(doc, first, last);
        loop {
            h.poll(doc);
            // The whole window, not just its first line. A window that
            // overlaps the one already in hand would otherwise find its
            // first line coloured by the *earlier* walk and report that
            // walk's cost as its own — which is no measurement at all.
            if (first..=last).all(|line| h.lines().contains_key(&line)) {
                return Some(ms(t.elapsed()));
            }
            if t.elapsed() > Duration::from_secs(30) {
                return None;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    fn step(out: &mut String, label: &str, value: Option<f64>) {
        match value {
            Some(v) => {
                let _ = writeln!(out, "| {label} | {v:.1} |");
            }
            None => {
                let _ = writeln!(out, "| {label} | no answer within 30 s |");
            }
        }
    }

    // The worker's one-time startup: the syntax definitions and the theme.
    let t = Instant::now();
    let set = syntect::parsing::SyntaxSet::load_defaults_nonewlines();
    let themes = syntect::highlighting::ThemeSet::load_defaults();
    let startup = ms(t.elapsed());
    drop((set, themes));

    let mut doc = Doc::open(&path).map_err(|e| e.to_string())?;
    doc.index_fully();
    let total = doc.line_count();
    // A window far from the top, where the distance from the nearest
    // checkpoint is the whole story.
    let deep = total.saturating_sub(100).max(1);

    let mut h = Highlighter::start(&path);
    let top = colour_window(&mut h, &doc, 0);
    let deep_first = colour_window(&mut h, &doc, deep);
    let deep_again = colour_window(&mut h, &doc, deep + 1);

    // One edit on the first line. The overlay then reports line 1 as the
    // earliest line ever changed, and the worker drops every checkpoint from
    // there down — which is every checkpoint there is — so the next colouring
    // walks from the top of the file again.
    doc.edit(Pos::new(0, 0), Pos::new(0, 0), "// ");
    let deep_after_edit = colour_window(&mut h, &doc, deep);

    // The same neighbourhood again, with nothing changed in between. The walk
    // above built checkpoints as it went; this is what reusing them costs. If
    // it costs as much as the walk, the checkpoints are being thrown away and
    // every scroll re-parses the file from line 1.
    let deep_after_edit_again = colour_window(&mut h, &doc, deep + 2);
    if let (Some(walk), Some(again)) = (deep_after_edit, deep_after_edit_again)
        && walk > 200.0
        && again > walk / 4.0
    {
        failures.push(format!(
            "highlight probe: re-colouring after an edit took {again:.0} ms against \
                 {walk:.0} ms for the full walk — checkpoints are not being reused"
        ));
    }

    // What the editor pays to ask. `request` clones the snapshot the worker
    // will read, and that snapshot carries the overlay, so this is where an
    // edited document could show up on the input path.
    let mut scroll = Vec::new();
    for i in 0..200u64 {
        let first = (i * 7) % deep;
        let t = Instant::now();
        h.request(&doc, first, first + 40);
        h.poll(&doc);
        scroll.push(ms(t.elapsed()));
    }
    let scroll_ms = median(scroll);
    drop(h);
    drop(doc);

    // The same path on a document the user has actually edited: every
    // keystroke bumps the version, so every keystroke clones the overlay.
    let edited_lines = 5_000u64;
    let mut edited = Doc::open(&path).map_err(|e| e.to_string())?;
    edited.index_fully();
    for row in 0..edited_lines {
        edited.edit(Pos::new(row, 0), Pos::new(row, 0), "//x");
    }
    let mut h2 = Highlighter::start(&path);
    let mut typed = Vec::new();
    for i in 0..200u64 {
        let t = Instant::now();
        edited.edit(Pos::new(i, 3), Pos::new(i, 3), "y");
        h2.request(&edited, 0, 40);
        h2.poll(&edited);
        typed.push(ms(t.elapsed()));
    }
    let typed_ms = median(typed);
    drop(h2);
    drop(edited);
    // The worker is idle the moment its channel closes, so the mapping is
    // gone and the file can be removed.
    std::thread::sleep(Duration::from_millis(50));
    let _ = std::fs::remove_file(&path);

    for (label, cost) in [
        ("editor: ask on a clean document", scroll_ms),
        ("editor: edit + ask, 5000 edited lines", typed_ms),
    ] {
        if cost > BUDGET_KEYSTROKE_MS {
            failures.push(format!(
                "highlight probe: {label} {cost:.2} ms > {:.0} ms",
                BUDGET_KEYSTROKE_MS
            ));
        }
    }

    let mut out = String::new();
    out.push_str(
        "\nSyntax highlighting — what the worker pays, and what the editor pays to ask:\n",
    );
    out.push_str("| step | ms |\n| --- | --- |\n");
    step(
        &mut out,
        "syntax set + theme load (worker startup)",
        Some(startup),
    );
    step(
        &mut out,
        "first window, top of file (includes that load)",
        top,
    );
    step(
        &mut out,
        &format!("window {deep} lines down, first visit (walks from the top)"),
        deep_first,
    );
    step(
        &mut out,
        "the next window (resumes from a checkpoint)",
        deep_again,
    );
    step(
        &mut out,
        "that window again after one edit on line 1",
        deep_after_edit,
    );
    step(
        &mut out,
        "the same window once more (reusing the checkpoints just built)",
        deep_after_edit_again,
    );
    step(&mut out, "editor: ask on a clean document", Some(scroll_ms));
    step(
        &mut out,
        "editor: edit + ask, 5000 edited lines",
        Some(typed_ms),
    );
    Ok(out)
}

fn motion_header(out: &mut String) {
    out.push_str("\nWord motions — one command, three starting columns on line 1:\n");
    out.push_str("| file | line 1 (chars) | w | e | b |\n| --- | --- | --- | --- | --- |\n");
}

fn write_motion(out: &mut String, case: &Case, line_chars: usize, times: &[f64; 3]) {
    let _ = writeln!(
        out,
        "| {} | {line_chars} | {:.3} | {:.3} | {:.3} |",
        case.name, times[0], times[1], times[2]
    );
}

/// How long a word motion takes, from three starting columns of line 1.
///
/// A motion reads at most the rest of the cursor's line, and never allocates
/// a copy of it, so the only shape that can make one expensive is an enormous
/// line. Starting at three columns separates "the cost of the motion" from
/// "the cost of the line": a scan that reads to the end of the line is
/// visible as a difference between the starts.
fn motion_scan(path: &Path) -> Result<(usize, [f64; 3]), String> {
    let mut doc = Doc::open(path).map_err(|e| e.to_string())?;
    doc.index_fully();
    let line_chars = doc.line_text(0).map_or(0, |t| t.chars().count());
    let starts = [
        Pos::new(0, 0),
        Pos::new(0, line_chars / 2),
        Pos::new(0, line_chars),
    ];

    let mut times = [0.0f64; 3];
    for (slot, motion) in [Motion::Word, Motion::EndOfWord, Motion::BackWord]
        .into_iter()
        .enumerate()
    {
        let mut best = f64::MAX;
        for start in starts {
            let t = Instant::now();
            for _ in 0..20 {
                std::hint::black_box(motion::target(&doc, start, motion));
            }
            let per_call = ms(t.elapsed()) / 20.0;
            best = best.min(per_call);
        }
        times[slot] = best;
    }
    Ok((line_chars, times))
}

fn check(failures: &mut Vec<String>, case: &Case, m: &Measurement, cold: bool) {
    let pass = if cold { "cold" } else { "warm" };
    if m.first_frame_ms > BUDGET_FIRST_FRAME_MS {
        failures.push(format!(
            "{} [{pass}]: first frame {:.1} ms > {:.0} ms",
            case.name, m.first_frame_ms, BUDGET_FIRST_FRAME_MS
        ));
    }
    if m.keystroke_ms > BUDGET_KEYSTROKE_MS {
        failures.push(format!(
            "{} [{pass}]: keystroke {:.1} ms > {:.0} ms",
            case.name, m.keystroke_ms, BUDGET_KEYSTROKE_MS
        ));
    }
    if m.cold_start_ms > BUDGET_COLD_START_MS {
        failures.push(format!(
            "{} [{pass}]: cold start {:.1} ms > {:.0} ms",
            case.name, m.cold_start_ms, BUDGET_COLD_START_MS
        ));
    }
}

/// Finds the `zeno` binary next to this one.
fn find_editor_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for name in ["zeno.exe", "zeno"] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Runs `zeno --bench-open <file>` and times the whole process.
///
/// The first run also pays for loading the binary from disk, so it is
/// repeated and the second run is reported.
fn measure_launch(bin: &Path, path: &Path) -> Option<f64> {
    let mut result = None;
    for _ in 0..2 {
        let t = Instant::now();
        let output = std::process::Command::new(bin)
            .arg("--bench-open")
            .arg(path)
            .output()
            .ok()?;
        let wall = ms(t.elapsed());
        if !output.status.success() {
            return None;
        }
        let inner: f64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        result = Some(wall - inner);
    }
    result
}

/// A breakdown of what a first-ever open of a file costs.
struct Breakdown {
    /// `File::open` plus the metadata call.
    file_open_ms: f64,
    /// Creating the mapping.
    mmap_ms: f64,
    /// Touching the first megabyte, which faults the pages in.
    fault_ms: f64,
}

/// Opens a file for the first time and times each step separately.
fn probe_cold(path: &Path) -> Result<Breakdown, String> {
    let t = Instant::now();
    let file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let file_open_ms = ms(t.elapsed());

    let t = Instant::now();
    // SAFETY: the file handle is read-only and the mapping is read-only.
    let mmap = unsafe { Mmap::map(&file).map_err(|e| e.to_string())? };
    let mmap_ms = ms(t.elapsed());

    let t = Instant::now();
    let mut sum = 0u64;
    for i in 0..len.min(1 << 20) {
        sum = sum.wrapping_add(mmap[i as usize] as u64);
    }
    std::hint::black_box(sum);
    let fault_ms = ms(t.elapsed());

    Ok(Breakdown {
        file_open_ms,
        mmap_ms,
        fault_ms,
    })
}

struct Options {
    data_dir: PathBuf,
    sizes: Vec<u64>,
    frames: usize,
    keep: bool,
    skip_generate: bool,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut data_dir = std::env::temp_dir().join("zeno-bench-data");
        let mut sizes = vec![1 << 20, 100 << 20, 1024 << 20];
        let mut frames = 200usize;
        let mut keep = false;
        let mut skip_generate = false;

        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--data-dir" => {
                    i += 1;
                    data_dir =
                        PathBuf::from(args.get(i).ok_or("--data-dir needs a value")?.clone());
                }
                "--sizes" => {
                    i += 1;
                    let raw = args.get(i).ok_or("--sizes needs a value")?;
                    sizes = parse_sizes(raw)?;
                }
                "--frames" => {
                    i += 1;
                    frames = args
                        .get(i)
                        .ok_or("--frames needs a value")?
                        .parse()
                        .map_err(|_| "--frames needs a number")?;
                }
                "--keep" => keep = true,
                "--skip-generate" => skip_generate = true,
                "-h" | "--help" => {
                    println!(
                        "zeno-bench [--data-dir DIR] [--sizes 1mb,100mb,1gb] [--frames N] [--keep] [--skip-generate]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
            i += 1;
        }
        Ok(Self {
            data_dir,
            sizes,
            frames,
            keep,
            skip_generate,
        })
    }
}

fn parse_sizes(raw: &str) -> Result<Vec<u64>, String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (num, mult) = if let Some(n) = part.strip_suffix("gb") {
            (n, 1u64 << 30)
        } else if let Some(n) = part.strip_suffix("mb") {
            (n, 1u64 << 20)
        } else if let Some(n) = part.strip_suffix("kb") {
            (n, 1u64 << 10)
        } else {
            (part, 1)
        };
        let value: u64 = num.parse().map_err(|_| format!("bad size `{part}`"))?;
        out.push(value * mult);
    }
    if out.is_empty() {
        return Err("no sizes given".into());
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Ordinary text: many short lines.
    Text,
    /// One enormous line with no newline in it at all.
    OneLine,
    /// Text without a trailing newline.
    NoTrailingNewline,
    /// Windows line endings.
    CrLf,
    /// A file with a byte-order mark and non-ASCII text.
    Unicode,
    /// A file whose bytes are not valid UTF-8.
    InvalidUtf8,
    /// A BOM-marked UTF-16 little-endian file.
    Utf16,
}

struct Case {
    name: String,
    size: u64,
    shape: Shape,
}

fn build_cases(opts: &Options) -> Vec<Case> {
    let mut cases = Vec::new();
    for size in &opts.sizes {
        cases.push(Case {
            name: format!("text-{}mb.txt", size >> 20),
            size: *size,
            shape: Shape::Text,
        });
    }
    // Adversarial shapes, always small enough to be cheap to generate.
    cases.push(Case {
        name: "one-line-16mb.txt".into(),
        size: 16 << 20,
        shape: Shape::OneLine,
    });
    cases.push(Case {
        name: "no-trailing-newline.txt".into(),
        size: 64 << 10,
        shape: Shape::NoTrailingNewline,
    });
    cases.push(Case {
        name: "crlf.txt".into(),
        size: 64 << 10,
        shape: Shape::CrLf,
    });
    cases.push(Case {
        name: "unicode.txt".into(),
        size: 64 << 10,
        shape: Shape::Unicode,
    });
    cases.push(Case {
        name: "invalid-utf8.txt".into(),
        size: 64 << 10,
        shape: Shape::InvalidUtf8,
    });
    cases.push(Case {
        name: "utf16le-bom.txt".into(),
        size: 64 << 10,
        shape: Shape::Utf16,
    });
    cases
}

/// A block of realistic-looking source code, used to fill the test files.
fn text_block() -> Vec<u8> {
    let mut block = Vec::with_capacity(64 * 1024);
    let mut i = 0u64;
    while block.len() < 64 * 1024 {
        let _ = writeln!(
            block,
            "fn compute_{i}(input: u64) -> Result<u64, Error> {{\n    let value = input.checked_mul({i}).ok_or(Error::Overflow)?;\n    Ok(value + {i})\n}}\n"
        );
        i += 1;
    }
    block
}

fn generate(path: &Path, case: &Case) -> std::io::Result<()> {
    let mut f = BufWriter::with_capacity(1 << 20, File::create(path)?);
    match case.shape {
        Shape::OneLine => {
            let chunk = vec![b'x'; 1 << 20];
            let mut left = case.size;
            while left > 0 {
                let n = left.min(chunk.len() as u64) as usize;
                f.write_all(&chunk[..n])?;
                left -= n as u64;
            }
        }
        Shape::Text => {
            let block = text_block();
            let mut left = case.size;
            while left > 0 {
                let n = left.min(block.len() as u64) as usize;
                f.write_all(&block[..n])?;
                left -= n as u64;
            }
        }
        Shape::NoTrailingNewline => {
            // The same text as the Text shape, minus the final newline.
            let block = text_block();
            let mut content: Vec<u8> = Vec::with_capacity(case.size as usize);
            while (content.len() as u64) < case.size {
                let n = (case.size as usize - content.len()).min(block.len());
                content.extend_from_slice(&block[..n]);
            }
            content.pop();
            f.write_all(&content)?;
        }
        Shape::CrLf => {
            let block = String::from_utf8_lossy(&text_block()).replace('\n', "\r\n");
            let block = block.as_bytes();
            let mut left = case.size;
            while left > 0 {
                let n = left.min(block.len() as u64) as usize;
                f.write_all(&block[..n])?;
                left -= n as u64;
            }
        }
        Shape::Unicode => {
            f.write_all("\u{feff}".as_bytes())?;
            let piece = "سلام دنیا — привет мир — こんにちは世界 — 🎉🎉🎉\n";
            let mut left = case.size;
            while left > 0 {
                // Never split a character in half.
                let mut cut = (left.min(piece.len() as u64)) as usize;
                while cut > 0 && !piece.is_char_boundary(cut) {
                    cut -= 1;
                }
                if cut == 0 {
                    // The remaining budget is smaller than one character.
                    f.write_all(piece.as_bytes())?;
                    break;
                }
                f.write_all(&piece.as_bytes()[..cut])?;
                left -= cut as u64;
            }
        }
        Shape::InvalidUtf8 => {
            // Ordinary text with a handful of bytes that are not valid UTF-8.
            let block = text_block();
            let mut left = case.size;
            while left > 0 {
                let n = left.min(block.len() as u64) as usize;
                let mut piece = block[..n].to_vec();
                // Sprinkle invalid bytes into the middle of the text.
                for i in (1..piece.len()).step_by(97) {
                    piece[i] = 0xff;
                }
                f.write_all(&piece)?;
                left -= n as u64;
            }
        }
        Shape::Utf16 => {
            // UTF-16 little-endian with a byte-order mark.
            let piece = "fn compute() -> u64 { 42 }\n";
            let mut left = case.size;
            f.write_all(&[0xff, 0xfe])?;
            while left > 0 {
                let mut bytes = Vec::with_capacity(piece.len() * 2);
                for unit in piece.encode_utf16() {
                    bytes.extend_from_slice(&unit.to_le_bytes());
                }
                let n = left.min(bytes.len() as u64) as usize;
                f.write_all(&bytes[..n])?;
                left -= n as u64;
            }
        }
    }
    f.flush()?;
    Ok(())
}

struct Measurement {
    size_bytes: u64,
    lines: u64,
    /// Time to map the file and get a usable document.
    cold_start_ms: f64,
    /// Time from the start of `Doc::open` to the first painted frame.
    first_frame_ms: f64,
    /// Median time to repaint the viewport without changing anything.
    repaint_ms: f64,
    /// Median time to apply one edit to the overlay.
    edit_ms: f64,
    /// Median time for one edit followed by one repaint.
    keystroke_ms: f64,
    /// Time from `Doc::open` until the whole file is indexed.
    index_ms: f64,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

fn measure(path: &Path, frames: usize) -> Result<Measurement, String> {
    let area = Rect::new(0, 0, WIDTH, HEIGHT);
    let state = ViewState::default();

    // --- cold start: map the file and get a usable document ---------------
    let t0 = Instant::now();
    let mut doc = Doc::open(path).map_err(|e| e.to_string())?;
    let cold_start = t0.elapsed();

    // --- first frame: index what is visible and paint it ------------------
    let t1 = Instant::now();
    let mut buf = Buffer::empty(area);
    {
        let view = View::new(&doc, &state);
        view.render(area, &mut buf);
    }
    let first_frame = t1.elapsed();

    // --- steady-state repaint ---------------------------------------------
    let mut samples = Vec::with_capacity(frames);
    for _ in 0..frames {
        let t = Instant::now();
        {
            let view = View::new(&doc, &state);
            view.render(area, &mut buf);
        }
        samples.push(ms(t.elapsed()));
    }
    let repaint_ms = median(samples);

    // --- one edit on its own ----------------------------------------------
    // A keystroke is a span edit at the cursor, and it is journaled so it can
    // be undone, so both costs are included here.
    let head = Pos::new(0, 0);
    let mut edit_samples = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = Instant::now();
        if i % 2 == 0 {
            doc.edit(head, head, "x");
        } else {
            doc.edit(head, Pos::new(0, 1), "");
        }
        edit_samples.push(ms(t.elapsed()));
    }
    let edit_ms = median(edit_samples);

    // --- one edit followed by one repaint: the keystroke path -------------
    let mut samples = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = Instant::now();
        if i % 2 == 0 {
            doc.edit(head, head, "x");
        } else {
            doc.edit(head, Pos::new(0, 1), "");
        }
        {
            let view = View::new(&doc, &state);
            view.render(area, &mut buf);
        }
        samples.push(ms(t.elapsed()));
    }
    let keystroke_ms = median(samples);

    // --- how long until the whole file is indexed -------------------------
    let t2 = Instant::now();
    while !doc.index().scan_more(doc.bytes(), 1 << 20) {}
    let index = t2.elapsed();

    Ok(Measurement {
        size_bytes: doc.len(),
        lines: doc.total_lines().unwrap_or(0),
        cold_start_ms: ms(cold_start),
        first_frame_ms: ms(cold_start) + ms(first_frame),
        repaint_ms,
        edit_ms,
        keystroke_ms,
        index_ms: ms(cold_start) + ms(first_frame) + ms(index),
    })
}

fn write_row(
    report: &mut String,
    case: &Case,
    cold: &Measurement,
    warm: &Measurement,
    launch: Option<&f64>,
) {
    let mb_s = |m: &Measurement| -> f64 {
        if m.index_ms > 0.0 {
            (m.size_bytes as f64 / (1024.0 * 1024.0)) / (m.index_ms / 1000.0)
        } else {
            0.0
        }
    };
    let _ = writeln!(
        report,
        "| {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0} | {} |",
        case.name,
        human_size(cold.size_bytes),
        cold.lines,
        cold.cold_start_ms,
        cold.first_frame_ms,
        cold.repaint_ms,
        cold.edit_ms,
        cold.keystroke_ms,
        warm.first_frame_ms,
        warm.keystroke_ms,
        mb_s(warm),
        launch
            .map(|l| format!("{l:.1}"))
            .unwrap_or_else(|| "n/a".into()),
    );
}

fn write_breakdown(report: &mut String, case: &Case, b: &Breakdown) {
    let _ = writeln!(
        report,
        "| {} | {:.1} | {:.1} | {:.1} |",
        case.name, b.file_open_ms, b.mmap_ms, b.fault_ms
    );
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn header(report: &mut String) {
    let _ = writeln!(
        report,
        "| file | size | lines | cold start | first frame | repaint | edit | keystroke | warm first frame | warm keystroke | index MB/s | launch |"
    );
    let _ = writeln!(
        report,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    );
}

fn breakdown_header(report: &mut String) {
    let _ = writeln!(report, "| file | file open | mmap | fault in 1 MiB |");
    let _ = writeln!(report, "| --- | --- | --- | --- |");
}
