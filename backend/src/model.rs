use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const ANSWER_STYLES: [(&str, &str); 4] = [
    ("triangle", "red"),
    ("diamond", "blue"),
    ("circle", "amber"),
    ("square", "green"),
];

fn default_reading_time() -> u64 {
    5
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamesFile {
    pub games: Vec<Game>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Game {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub questions: Vec<Question>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    deny_unknown_fields
)]
pub enum Question {
    #[serde(rename = "multiple_choice")]
    MultipleChoice {
        id: String,
        prompt: String,
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        image_alt: Option<String>,
        time_limit_seconds: u64,
        #[serde(default = "default_reading_time")]
        reading_time_seconds: u64,
        #[serde(default)]
        double_points: bool,
        options: Vec<Choice>,
        correct_option_id: String,
    },
    #[serde(rename = "free_text")]
    FreeText {
        id: String,
        prompt: String,
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        image_alt: Option<String>,
        time_limit_seconds: u64,
        #[serde(default = "default_reading_time")]
        reading_time_seconds: u64,
        #[serde(default)]
        double_points: bool,
        accepted_answers: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub text: String,
}

impl Question {
    pub fn id(&self) -> &str {
        match self {
            Self::MultipleChoice { id, .. } | Self::FreeText { id, .. } => id,
        }
    }

    pub fn time_limit_seconds(&self) -> u64 {
        match self {
            Self::MultipleChoice {
                time_limit_seconds, ..
            }
            | Self::FreeText {
                time_limit_seconds, ..
            } => *time_limit_seconds,
        }
    }

    pub fn reading_time_seconds(&self) -> u64 {
        match self {
            Self::MultipleChoice {
                reading_time_seconds,
                ..
            }
            | Self::FreeText {
                reading_time_seconds,
                ..
            } => *reading_time_seconds,
        }
    }

    pub fn double_points(&self) -> bool {
        match self {
            Self::MultipleChoice { double_points, .. } | Self::FreeText { double_points, .. } => {
                *double_points
            }
        }
    }

    pub fn image(&self) -> Option<&str> {
        match self {
            Self::MultipleChoice { image, .. } | Self::FreeText { image, .. } => image.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "name")]
pub enum Phase {
    #[default]
    Selection,
    Lobby {
        game_id: String,
    },
    Reading {
        question_index: usize,
        deadline_ms: i64,
    },
    Answering {
        question_index: usize,
        started_at_ms: i64,
        deadline_ms: i64,
    },
    Reveal {
        question_index: usize,
    },
    Leaderboard {
        question_index: usize,
    },
    FinalLeaderboard,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: String,
    pub username: String,
    pub token_hash: String,
    pub score: i64,
    pub correct_count: u32,
    pub correct_response_ms: u64,
    pub joined_order: u64,
    pub eligible_from_question: usize,
    pub previous_rank: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum SubmittedAnswer {
    MultipleChoice { option_id: String },
    FreeText { text: String },
}

#[derive(Clone, Debug)]
pub struct Submission {
    pub player_id: String,
    pub question_index: usize,
    pub answer: SubmittedAnswer,
    pub response_ms: u64,
    pub correct: Option<bool>,
    pub points: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct GameState {
    pub revision: u64,
    pub host_token_hash: Option<String>,
    pub phase: Phase,
    pub active_game: Option<Game>,
    pub players: BTreeMap<String, Player>,
    pub submissions: Vec<Submission>,
    pub next_join_order: u64,
    pub advertised_url: Option<String>,
    pub network_warning: Option<String>,
}
