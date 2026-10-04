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
}
