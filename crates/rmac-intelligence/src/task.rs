//! The closed list of tasks the service runs (ADR 0024 §4). Callers name a
//! task; they never send a prompt. Phase 1 implements `intent` only; the
//! other ADR tasks (proofread, rewrite, summarise, explain_command,
//! help_answer, plan) arrive with their features and are rejected until then.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Task {
    /// One Spotlight sentence to one typed [`crate::Intent`].
    Intent,
}

impl Task {
    pub const ALL: [Task; 1] = [Task::Intent];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|task| task.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_tasks_parse() {
        assert_eq!(Task::parse("intent"), Some(Task::Intent));
        for refused in [
            "",
            "Intent",
            "intent ",
            "prompt",
            "chat",
            "proofread",
            "summarise",
            "plan",
        ] {
            assert_eq!(Task::parse(refused), None, "{refused:?}");
        }
    }
}
