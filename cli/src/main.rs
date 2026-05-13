// `vlerv` CLI shim. Translates `vlerv open <path>` into
// `open "vlerv://open?path=<abs>"` and `vlerv reveal <path>` into
// `open "vlerv://reveal?path=<abs>"`. C3: reveal verb implemented.

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use std::path::Path;
use std::process::{Command, ExitCode};

/// Characters to percent-encode in a path query parameter value.
/// Encode everything except unreserved characters (RFC 3986): A-Z a-z 0-9 - _ . ~
const PATH_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn print_usage() {
    eprintln!("Usage: vlerv open <path>");
    eprintln!("       vlerv reveal <path>");
    eprintln!();
    eprintln!("Supported verbs: open, reveal");
}

fn resolve_path(raw_path: &str) -> std::path::PathBuf {
    let p = Path::new(raw_path);
    let abs_path = if p.is_absolute() {
        p.to_path_buf()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        cwd.join(p)
    };

    // Canonicalize to resolve any symlinks / `.` / `..`.
    abs_path.canonicalize().unwrap_or(abs_path)
}

fn open_url(url: &str) -> ExitCode {
    let status = Command::new("open").arg(url).status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => {
            eprintln!("vlerv: `open` exited with status {s}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("vlerv: failed to spawn `open`: {e}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return ExitCode::from(1);
    }

    match args[1].as_str() {
        "open" => {
            if args.len() < 3 {
                eprintln!("Usage: vlerv open <path>");
                eprintln!("Error: missing path argument for 'open' verb");
                return ExitCode::from(1);
            }

            let canonical = resolve_path(&args[2]);
            let path_str = canonical.to_string_lossy();
            let encoded = utf8_percent_encode(&path_str, PATH_SET).to_string();
            let url = format!("vlerv://open?path={encoded}");
            open_url(&url)
        }

        "reveal" => {
            if args.len() < 3 {
                eprintln!("Usage: vlerv reveal <path>");
                eprintln!("Error: missing path argument for 'reveal' verb");
                return ExitCode::from(1);
            }

            let canonical = resolve_path(&args[2]);
            let path_str = canonical.to_string_lossy();
            let encoded = utf8_percent_encode(&path_str, PATH_SET).to_string();
            let url = format!("vlerv://reveal?path={encoded}");
            open_url(&url)
        }

        other => {
            eprintln!("vlerv: unknown verb '{other}'");
            print_usage();
            ExitCode::from(1)
        }
    }
}
