//! The closed list of tasks the service runs (ADR 0024 §4). Callers name a
//! task; they never send a prompt. `intent` is Spotlight's; the Writing
//! Tools tasks (phase 2) are Notes', Text Editor's and Mail's. The other
//! ADR tasks (explain_command, help_answer, plan) arrive with their
//! features and are rejected until then.

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Task {
    /// One Spotlight sentence to one typed [`crate::Intent`].
    Intent,
    /// Writing Tools ▸ Proofread: fix spelling, grammar and punctuation only.
    Proofread,
    /// Writing Tools ▸ Rewrite: the same meaning in different words.
    Rewrite,
    /// Writing Tools ▸ Make Friendly.
    Friendly,
    /// Writing Tools ▸ Make Professional.
    Professional,
    /// Writing Tools ▸ Make Concise.
    Concise,
    /// Writing Tools ▸ Summarise: a short paragraph.
    Summarise,
    /// Writing Tools ▸ Create Key Points: a short bulleted list.
    KeyPoints,
    /// Writing Tools ▸ Make List: the text as a bulleted list.
    List,
}

impl Task {
    pub const ALL: [Task; 9] = [
        Task::Intent,
        Task::Proofread,
        Task::Rewrite,
        Task::Friendly,
        Task::Professional,
        Task::Concise,
        Task::Summarise,
        Task::KeyPoints,
        Task::List,
    ];

    /// The Writing Tools tasks, in their menu order.
    pub const WRITING: [Task; 8] = [
        Task::Proofread,
        Task::Rewrite,
        Task::Friendly,
        Task::Professional,
        Task::Concise,
        Task::Summarise,
        Task::KeyPoints,
        Task::List,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|task| task.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Proofread => "proofread",
            Self::Rewrite => "rewrite",
            Self::Friendly => "friendly",
            Self::Professional => "professional",
            Self::Concise => "concise",
            Self::Summarise => "summarise",
            Self::KeyPoints => "key_points",
            Self::List => "list",
        }
    }

    /// A Writing Tools task: streamed text out, through `Write`.
    pub fn is_writing(self) -> bool {
        self != Self::Intent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_tasks_parse() {
        assert_eq!(Task::parse("intent"), Some(Task::Intent));
        assert_eq!(Task::parse("proofread"), Some(Task::Proofread));
        assert_eq!(Task::parse("key_points"), Some(Task::KeyPoints));
        for task in Task::ALL {
            assert_eq!(Task::parse(task.as_str()), Some(task));
        }
        for refused in [
            "",
            "Intent",
            "intent ",
            "prompt",
            "chat",
            "summarize",
            "Proofread",
            "explain_command",
            "help_answer",
            "plan",
            "compose",
        ] {
            assert_eq!(Task::parse(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn writing_tasks_are_every_task_but_intent() {
        assert!(!Task::Intent.is_writing());
        assert_eq!(
            Task::ALL.iter().filter(|task| task.is_writing()).count(),
            Task::WRITING.len()
        );
        assert!(Task::WRITING.iter().all(|task| task.is_writing()));
    }
}
