//! `-e PROGRAM ARGS…`: the POSIX `x-terminal-emulator` convention. The
//! terminal `exec`s PROGRAM with ARGS verbatim — no shell reads them, so no
//! word-splitting, globbing, quote removal or `$VAR` expansion happens;
//! whatever argv the caller built is handed to the kernel unchanged. `-e`
//! consumes every argument after it, including ones that look like more
//! flags, as PROGRAM and its ARGS — it must be the last option, exactly as
//! `xterm -e`/`x-terminal-emulator -e` behave.

/// A program to `exec` directly in a freshly opened session instead of the
/// user's shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// Shell ▸ New Command… with "Run command inside a shell" unchecked: split
/// the typed line into a program and its arguments without invoking a
/// shell, so no `$VAR` expansion, globbing or pipeline runs — only single
/// and double quoting (no nested escapes) to let one argument contain
/// spaces. An unterminated quote is treated as running to the end of the
/// string, same as a forgiving single-line form field should behave.
pub(crate) fn split_command_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for character in input.chars() {
        match quote {
            Some(q) if character == q => quote = None,
            Some(_) => current.push(character),
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                in_word = true;
            }
            None if character.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            None => {
                current.push(character);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

/// Shell ▸ New Command…: build the exec-style argv for the typed command,
/// honouring "Run command inside a shell" the way the Mac's own checkbox
/// does — checked wraps it in `/bin/sh -c` (pipes, globs, `$VAR`s all
/// work); unchecked runs the first word directly with [`split_command_words`].
pub(crate) fn command_to_exec(command: &str, run_in_shell: bool) -> Option<ExecCommand> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    if run_in_shell {
        return Some(shell_c_command(trimmed));
    }
    let mut words = split_command_words(trimmed);
    if words.is_empty() {
        return None;
    }
    let program = words.remove(0);
    Some(ExecCommand {
        program,
        args: words,
    })
}

/// `-c`/`-Command`: run `line` through a shell rather than `exec`ing it
/// directly. `/bin/sh` on Unix; PowerShell (ADR 0023 phase 2's Windows
/// default shell) on Windows, since there is no `cmd.exe` equivalent of a
/// POSIX pipeline.
#[cfg(unix)]
fn shell_c_command(line: &str) -> ExecCommand {
    ExecCommand {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), line.to_string()],
    }
}

#[cfg(windows)]
fn shell_c_command(line: &str) -> ExecCommand {
    ExecCommand {
        program: "powershell.exe".to_string(),
        args: vec!["-Command".to_string(), line.to_string()],
    }
}

/// Why `-e` was present but unusable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecFlagError {
    /// `-e` was the last argument, with no program after it.
    MissingProgram,
}

impl std::fmt::Display for ExecFlagError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingProgram => "-e requires a program to run",
        })
    }
}

/// Application ▸ Quit and Keep Windows (TERM-22): the prefix on the one
/// argument that carries a whole restored window — see
/// `session_restore::RestoreWindow`. A window opened this way is handed to
/// `rmac_ui::boot_app_instance` exactly like any other (one argument list
/// per window), so the restore itself needs no extra IPC.
const RESTORE_PREFIX: &str = "--restore=";

/// Shell ▸ Open…/Edit Background Colour and ordinary launches never set
/// this; only a relaunch after Quit and Keep Windows does.
pub(crate) fn parse_restore_flag(args: &[String]) -> Option<crate::session_restore::RestoreWindow> {
    args.iter()
        .find_map(|arg| arg.strip_prefix(RESTORE_PREFIX))
        .and_then(crate::session_restore::decode)
}

/// The one argument that reopens `window`, for `rmac_ui::open_another_window`
/// (while already running) or the next launch's argument list (after Quit
/// and Keep Windows). `None` only if `window` somehow fails to serialize.
pub(crate) fn restore_flag(window: &crate::session_restore::RestoreWindow) -> Option<String> {
    crate::session_restore::encode(window).map(|json| format!("{RESTORE_PREFIX}{json}"))
}

/// Looks for `-e PROGRAM ARGS…` in a process's own argv (already without
/// argv[0]). `Ok(None)` means there was no `-e` at all — the caller falls
/// back to an interactive shell. Every argument from the first `-e` onward
/// belongs to PROGRAM, never to Terminal itself, so a later `-e` or any
/// flag-looking string in ARGS is passed through unexamined.
pub(crate) fn parse_exec_flag(args: &[String]) -> Result<Option<ExecCommand>, ExecFlagError> {
    let Some(position) = args.iter().position(|arg| arg == "-e") else {
        return Ok(None);
    };
    let rest = &args[position + 1..];
    let Some((program, rest_args)) = rest.split_first() else {
        return Err(ExecFlagError::MissingProgram);
    };
    Ok(Some(ExecCommand {
        program: program.clone(),
        args: rest_args.to_vec(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn no_flag_falls_back_to_a_shell() {
        assert_eq!(parse_exec_flag(&args(&[])), Ok(None));
        assert_eq!(parse_exec_flag(&args(&["--profile=1"])), Ok(None));
    }

    #[test]
    fn execs_the_program_with_its_arguments_verbatim() {
        assert_eq!(
            parse_exec_flag(&args(&["-e", "vim", "notes.txt"])),
            Ok(Some(ExecCommand {
                program: "vim".into(),
                args: vec!["notes.txt".into()],
            }))
        );
        assert_eq!(
            parse_exec_flag(&args(&["-e", "/usr/bin/top"])),
            Ok(Some(ExecCommand {
                program: "/usr/bin/top".into(),
                args: vec![],
            }))
        );
    }

    #[test]
    fn everything_after_e_belongs_to_the_program_not_terminal() {
        // A flag-looking argument, and even a second `-e`, are the
        // program's own argv once `-e` has been seen — never re-parsed.
        assert_eq!(
            parse_exec_flag(&args(&["-e", "grep", "-e", "needle", "file"])),
            Ok(Some(ExecCommand {
                program: "grep".into(),
                args: vec!["-e".into(), "needle".into(), "file".into()],
            }))
        );
    }

    #[test]
    fn a_flag_before_e_is_still_terminals_own() {
        assert_eq!(
            parse_exec_flag(&args(&["--profile=2", "-e", "bash", "-lc", "echo hi"])),
            Ok(Some(ExecCommand {
                program: "bash".into(),
                args: vec!["-lc".into(), "echo hi".into()],
            }))
        );
    }

    #[test]
    fn e_with_nothing_after_it_is_a_usage_error() {
        assert_eq!(
            parse_exec_flag(&args(&["-e"])),
            Err(ExecFlagError::MissingProgram)
        );
        assert_eq!(
            parse_exec_flag(&args(&["--profile=1", "-e"])),
            Err(ExecFlagError::MissingProgram)
        );
    }

    #[test]
    fn splits_plain_words_on_whitespace() {
        assert_eq!(
            split_command_words("top -o cpu"),
            vec!["top".to_string(), "-o".to_string(), "cpu".to_string()]
        );
        assert_eq!(split_command_words("   "), Vec::<String>::new());
    }

    #[test]
    fn quoted_words_keep_their_inner_spaces() {
        assert_eq!(
            split_command_words(r#"echo "hello world" 'a b'"#),
            vec![
                "echo".to_string(),
                "hello world".to_string(),
                "a b".to_string()
            ]
        );
        // An unterminated quote runs to the end rather than erroring.
        assert_eq!(
            split_command_words(r#"echo "unterminated"#),
            vec!["echo".to_string(), "unterminated".to_string()]
        );
    }

    #[test]
    fn restore_flag_round_trips_through_argv() {
        use crate::session_restore::{RestoreTab, RestoreWindow};

        let window = RestoreWindow {
            tabs: vec![RestoreTab {
                cwd: Some(std::path::PathBuf::from("/home/user")),
                program: None,
                args: Vec::new(),
                profile: 3,
                scrollback: "hi\n".to_string(),
            }],
        };
        let flag = restore_flag(&window).expect("encodes");
        assert!(flag.starts_with("--restore="));
        let parsed = parse_restore_flag(&args(&["--profile=1", &flag]));
        assert_eq!(parsed, Some(window));
    }

    #[test]
    fn no_restore_flag_is_none() {
        assert_eq!(parse_restore_flag(&args(&["--profile=1"])), None);
        assert_eq!(parse_restore_flag(&args(&[])), None);
    }

    #[test]
    fn command_to_exec_wraps_in_a_shell_only_when_asked() {
        assert_eq!(
            command_to_exec("ls -la ~", false),
            Some(ExecCommand {
                program: "ls".into(),
                args: vec!["-la".into(), "~".into()],
            })
        );
        assert_eq!(
            command_to_exec("ls -la | grep foo", true),
            Some(shell_c_command("ls -la | grep foo"))
        );
        assert_eq!(command_to_exec("   ", false), None);
        assert_eq!(command_to_exec("", true), None);
    }
}
