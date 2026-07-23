use std::collections::HashMap;

use serde_json::{Value, json};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use uuid::Uuid;

use crate::{
    content::Catalog,
    model::{ANSWER_STYLES, Game, GameState, Phase, Player, Question, Submission, SubmittedAnswer},
};

pub struct Engine {
    pub state: GameState,
    pub catalog: Catalog,
    pub connected_players: HashMap<String, usize>,
    pub host_connections: usize,
    pub join_urls: Vec<String>,
}

impl Engine {
    pub fn new(state: GameState, catalog: Catalog, join_urls: Vec<String>) -> Self {
        Self {
            state,
            catalog,
            connected_players: HashMap::new(),
            host_connections: 0,
            join_urls,
        }
    }

    pub fn bump(&mut self) {
        self.state.revision = self.state.revision.saturating_add(1);
    }

    pub fn player_by_token_hash(&self, token_hash: &str) -> Option<&Player> {
        self.state
            .players
            .values()
            .find(|player| player.token_hash == token_hash)
    }

    pub fn join_player(&mut self, username: &str, token_hash: String) -> Result<String, String> {
        let username = username.trim();
        let length = username.chars().count();
        if !(1..=24).contains(&length) || username.chars().any(char::is_control) {
            return Err("Use a username between 1 and 24 characters.".into());
        }
        if self
            .state
            .players
            .values()
            .any(|player| player.username.to_lowercase() == username.to_lowercase())
        {
            return Err("That username is already in use on this Hoot.".into());
        }
        let eligible_from_question = match self.current_question_index() {
            Some(index) => index + 1,
            None => 0,
        };
        let id = Uuid::new_v4().to_string();
        let player = Player {
            id: id.clone(),
            username: username.to_owned(),
            token_hash,
            score: 0,
            correct_count: 0,
            correct_response_ms: 0,
            joined_order: self.state.next_join_order,
            eligible_from_question,
            previous_rank: None,
        };
        self.state.next_join_order += 1;
        self.state.players.insert(id.clone(), player);
        self.bump();
        Ok(id)
    }

    pub fn select_game(&mut self, game_id: &str) -> Result<(), String> {
        if !matches!(
            self.state.phase,
            Phase::Selection | Phase::Lobby { .. } | Phase::FinalLeaderboard
        ) {
            return Err("Finish the current game before selecting another.".into());
        }
        if self.catalog.game(game_id).is_none() {
            return Err("That game is not available.".into());
        }
        self.state.active_game = None;
        self.state.submissions.clear();
        for player in self.state.players.values_mut() {
            player.score = 0;
            player.correct_count = 0;
            player.correct_response_ms = 0;
            player.previous_rank = None;
            player.eligible_from_question = 0;
        }
        self.state.phase = Phase::Lobby {
            game_id: game_id.to_owned(),
        };
        self.bump();
        Ok(())
    }

    pub fn start_game(&mut self, now_ms: i64) -> Result<(), String> {
        let Phase::Lobby { game_id } = &self.state.phase else {
            return Err("A game can only start from the lobby.".into());
        };
        if self.state.players.is_empty() {
            return Err("At least one player must join before starting.".into());
        }
        let game = self
            .catalog
            .game(game_id)
            .cloned()
            .ok_or("The selected game is no longer available.")?;
        let deadline_ms = now_ms + game.questions[0].reading_time_seconds() as i64 * 1_000;
        self.state.active_game = Some(game);
        self.state.phase = Phase::Reading {
            question_index: 0,
            deadline_ms,
        };
        self.bump();
        Ok(())
    }

    pub fn advance_host(&mut self, now_ms: i64) -> Result<(), String> {
        match self.state.phase.clone() {
            Phase::Reading { question_index, .. } => {
                let duration = self.active_game()?.questions[question_index].time_limit_seconds()
                    as i64
                    * 1_000;
                self.state.phase = Phase::Answering {
                    question_index,
                    started_at_ms: now_ms,
                    deadline_ms: now_ms + duration,
                };
            }
            Phase::Answering { question_index, .. } => {
                self.finish_answering(question_index)?;
                self.state.phase = Phase::Reveal { question_index };
            }
            Phase::Reveal { question_index } => {
                let last = self.active_game()?.questions.len() == question_index + 1;
                self.state.phase = if last {
                    Phase::FinalLeaderboard
                } else {
                    Phase::Leaderboard { question_index }
                };
            }
            Phase::Leaderboard { question_index } => {
                self.commit_current_ranks();
                let next = question_index + 1;
                let reading_ms =
                    self.active_game()?.questions[next].reading_time_seconds() as i64 * 1_000;
                self.state.phase = Phase::Reading {
                    question_index: next,
                    deadline_ms: now_ms + reading_ms,
                };
            }
            _ => return Err("There is nothing for the host to advance right now.".into()),
        }
        self.bump();
        Ok(())
    }

    pub fn tick(&mut self, now_ms: i64) -> Result<bool, String> {
        let mut changed = false;
        loop {
            match self.state.phase.clone() {
                Phase::Reading {
                    question_index,
                    deadline_ms,
                } if now_ms >= deadline_ms => {
                    let duration = self.active_game()?.questions[question_index]
                        .time_limit_seconds() as i64
                        * 1_000;
                    self.state.phase = Phase::Answering {
                        question_index,
                        started_at_ms: deadline_ms,
                        deadline_ms: deadline_ms + duration,
                    };
                    self.bump();
                    changed = true;
                }
                Phase::Answering {
                    question_index,
                    deadline_ms,
                    ..
                } if now_ms >= deadline_ms => {
                    self.finish_answering(question_index)?;
                    self.state.phase = Phase::Reveal { question_index };
                    self.bump();
                    changed = true;
                }
                _ => break,
            }
        }
        Ok(changed)
    }

    pub fn submit(
        &mut self,
        player_id: &str,
        answer: SubmittedAnswer,
        now_ms: i64,
    ) -> Result<(), String> {
        let Phase::Answering {
            question_index,
            started_at_ms,
            deadline_ms,
        } = self.state.phase
        else {
            return Err("Answers are not open.".into());
        };
        if now_ms >= deadline_ms {
            return Err("Time is up for this question.".into());
        }
        let player = self
            .state
            .players
            .get(player_id)
            .ok_or("Player session was not found.")?;
        if player.eligible_from_question > question_index {
            return Err("You can start answering on the next question.".into());
        }
        if self
            .state
            .submissions
            .iter()
            .any(|item| item.player_id == player_id && item.question_index == question_index)
        {
            return Err("Your first answer is already locked in.".into());
        }
        let question = &self.active_game()?.questions[question_index];
        match (&answer, question) {
            (
                SubmittedAnswer::MultipleChoice { option_id },
                Question::MultipleChoice { options, .. },
            ) if options.iter().any(|option| option.id == *option_id) => {}
            (SubmittedAnswer::FreeText { text }, Question::FreeText { .. })
                if !text.trim().is_empty() && text.chars().count() <= 120 => {}
            (SubmittedAnswer::FreeText { .. }, Question::FreeText { .. }) => {
                return Err("Enter an answer up to 120 characters.".into());
            }
            _ => return Err("That answer is not valid for this question.".into()),
        }
        self.state.submissions.push(Submission {
            player_id: player_id.to_owned(),
            question_index,
            answer,
            response_ms: now_ms.saturating_sub(started_at_ms) as u64,
            correct: None,
            points: None,
        });
        if self.all_eligible_players_answered(question_index) {
            self.finish_answering(question_index)?;
            self.state.phase = Phase::Reveal { question_index };
        }
        self.bump();
        Ok(())
    }

    fn all_eligible_players_answered(&self, question_index: usize) -> bool {
        let eligible = self
            .state
            .players
            .values()
            .filter(|player| player.eligible_from_question <= question_index)
            .count();
        if eligible == 0 {
            return false;
        }
        let submitted = self
            .state
            .submissions
            .iter()
            .filter(|item| item.question_index == question_index)
            .count();
        submitted >= eligible
    }

    pub fn set_advertised_url(&mut self, url: String) -> Result<(), String> {
        if !self.join_urls.contains(&url) {
            return Err("Choose one of the detected join URLs.".into());
        }
        self.state.advertised_url = Some(url);
        self.bump();
        Ok(())
    }

    pub fn reload_catalog(&mut self) -> Result<(), String> {
        if !matches!(
            self.state.phase,
            Phase::Selection | Phase::Lobby { .. } | Phase::FinalLeaderboard
        ) {
            return Err("Content can only be reloaded between games.".into());
        }
        let catalog = self.catalog.reload().map_err(|error| error.to_string())?;
        if let Phase::Lobby { game_id } = &self.state.phase
            && catalog.game(game_id).is_none()
        {
            self.state.phase = Phase::Selection;
        }
        self.catalog = catalog;
        self.bump();
        Ok(())
    }

    pub fn host_snapshot(&self, server_time_ms: i64) -> Value {
        let rankings = self.rankings();
        let game_summaries = self
            .catalog
            .games
            .iter()
            .map(|game| {
                json!({
                    "id": game.id,
                    "title": game.title,
                    "description": game.description,
                    "questionCount": game.questions.len(),
                })
            })
            .collect::<Vec<_>>();
        let roster = rankings
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "username": entry.username,
                    "score": entry.score,
                    "rank": entry.rank,
                    "rankDelta": entry.rank_delta,
                    "connected": self.connected_players.get(&entry.id).copied().unwrap_or(0) > 0,
                })
            })
            .collect::<Vec<_>>();
        let current_question = self.current_question().map(host_question_json);
        json!({
            "role": "host",
            "revision": self.state.revision,
            "serverTimeMs": server_time_ms,
            "phase": self.state.phase,
            "games": game_summaries,
            "players": roster,
            "question": current_question,
            "questionNumber": self.current_question_index().map(|index| index + 1),
            "questionCount": self.state.active_game.as_ref().map(|game| game.questions.len()),
            "distribution": self.current_question_index().and_then(|index| self.distribution(index)),
            "joinUrls": self.join_urls,
            "joinUrl": self.state.advertised_url.clone().or_else(|| self.join_urls.first().cloned()),
            "networkWarning": self.state.network_warning.clone(),
            "hostConnected": self.host_connections > 0,
        })
    }

    pub fn player_snapshot(&self, player_id: &str, server_time_ms: i64) -> Result<Value, String> {
        let player = self
            .state
            .players
            .get(player_id)
            .ok_or("Player session was not found.")?;
        let current_index = self.current_question_index();
        let eligible = current_index.is_none_or(|index| player.eligible_from_question <= index);
        let submission = current_index.and_then(|index| {
            self.state
                .submissions
                .iter()
                .find(|item| item.player_id == player_id && item.question_index == index)
        });
        let controls = match (&self.state.phase, self.current_question(), eligible) {
            (Phase::Answering { .. }, Some(Question::MultipleChoice { options, .. }), true) => {
                Some(json!({
                    "kind": "multiple_choice",
                    "options": options.iter().enumerate().map(|(index, option)| json!({
                        "id": option.id,
                        "number": index + 1,
                        "shape": ANSWER_STYLES[index].0,
                        "color": ANSWER_STYLES[index].1,
                    })).collect::<Vec<_>>()
                }))
            }
            (Phase::Answering { .. }, Some(Question::FreeText { .. }), true) => {
                Some(json!({ "kind": "free_text" }))
            }
            _ => None,
        };
        let result = match (&self.state.phase, submission) {
            (
                Phase::Reveal { .. } | Phase::Leaderboard { .. } | Phase::FinalLeaderboard,
                Some(item),
            ) => Some(json!({
                "correct": item.correct,
                "points": item.points,
                "answer": answer_result_json(&item.answer, self.current_question()),
            })),
            _ => None,
        };
        let rankings = self.rankings();
        let ranking = rankings.iter().find(|entry| entry.id == player_id);
        let ahead = ranking.and_then(|entry| {
            if entry.rank <= 1 {
                return None;
            }
            rankings
                .iter()
                .find(|other| other.rank + 1 == entry.rank)
                .map(|other| {
                    json!({
                        "username": other.username,
                        "gap": other.score - entry.score,
                    })
                })
        });
        Ok(json!({
            "role": "player",
            "revision": self.state.revision,
            "serverTimeMs": server_time_ms,
            "phase": self.state.phase,
            "self": {
                "id": player.id,
                "username": player.username,
                "score": player.score,
                "rank": ranking.map(|entry| entry.rank),
                "rankDelta": ranking.map(|entry| entry.rank_delta),
            },
            "playerCount": self.state.players.len(),
            "eligible": eligible,
            "submitted": submission.is_some(),
            "controls": controls,
            "result": result,
            "questionNumber": self.current_question_index().map(|index| index + 1),
            "questionCount": self.state.active_game.as_ref().map(|game| game.questions.len()),
            "ahead": ahead,
        }))
    }

    pub fn current_question_index(&self) -> Option<usize> {
        match self.state.phase {
            Phase::Reading { question_index, .. }
            | Phase::Answering { question_index, .. }
            | Phase::Reveal { question_index }
            | Phase::Leaderboard { question_index } => Some(question_index),
            Phase::FinalLeaderboard => self
                .state
                .active_game
                .as_ref()?
                .questions
                .len()
                .checked_sub(1),
            Phase::Selection | Phase::Lobby { .. } => None,
        }
    }

    fn current_question(&self) -> Option<&Question> {
        self.state
            .active_game
            .as_ref()?
            .questions
            .get(self.current_question_index()?)
    }

    fn active_game(&self) -> Result<&Game, String> {
        self.state
            .active_game
            .as_ref()
            .ok_or_else(|| "The active game could not be restored.".into())
    }

    fn finish_answering(&mut self, question_index: usize) -> Result<(), String> {
        let question = self.active_game()?.questions[question_index].clone();
        let limit_ms = question.time_limit_seconds() * 1_000;
        for submission in self
            .state
            .submissions
            .iter_mut()
            .filter(|item| item.question_index == question_index)
        {
            let correct = answer_is_correct(&question, &submission.answer);
            let points = if correct {
                let ratio = (submission.response_ms.min(limit_ms) as f64) / (limit_ms as f64);
                let base = (1_000.0 * (1.0 - 0.5 * ratio)).round() as i64;
                base * if question.double_points() { 2 } else { 1 }
            } else {
                0
            };
            submission.correct = Some(correct);
            submission.points = Some(points);
            if let Some(player) = self.state.players.get_mut(&submission.player_id) {
                player.score += points;
                if correct {
                    player.correct_count += 1;
                    player.correct_response_ms += submission.response_ms;
                }
            }
        }
        Ok(())
    }

    fn rankings(&self) -> Vec<Ranking> {
        let mut players = self.state.players.values().collect::<Vec<_>>();
        players.sort_by_key(|player| {
            (
                std::cmp::Reverse(player.score),
                std::cmp::Reverse(player.correct_count),
                player.correct_response_ms,
                player.joined_order,
            )
        });
        players
            .into_iter()
            .enumerate()
            .map(|(index, player)| {
                let rank = index + 1;
                Ranking {
                    id: player.id.clone(),
                    username: player.username.clone(),
                    score: player.score,
                    rank,
                    rank_delta: player
                        .previous_rank
                        .map(|old| old as i64 - rank as i64)
                        .unwrap_or(0),
                }
            })
            .collect()
    }

    fn commit_current_ranks(&mut self) {
        for ranking in self.rankings() {
            if let Some(player) = self.state.players.get_mut(&ranking.id) {
                player.previous_rank = Some(ranking.rank);
            }
        }
    }

    fn distribution(&self, index: usize) -> Option<Value> {
        let question = self.state.active_game.as_ref()?.questions.get(index)?;
        let submissions = self
            .state
            .submissions
            .iter()
            .filter(|item| item.question_index == index)
            .collect::<Vec<_>>();
        match question {
            Question::MultipleChoice {
                options,
                correct_option_id,
                ..
            } => Some(json!({
                "kind": "multiple_choice",
                "options": options.iter().enumerate().map(|(slot, option)| json!({
                    "id": option.id,
                    "text": option.text,
                    "count": submissions.iter().filter(|item| matches!(&item.answer, SubmittedAnswer::MultipleChoice { option_id } if option_id == &option.id)).count(),
                    "correct": option.id == *correct_option_id,
                    "number": slot + 1,
                    "shape": ANSWER_STYLES[slot].0,
                    "color": ANSWER_STYLES[slot].1,
                })).collect::<Vec<_>>(),
                "unanswered": self.state.players.values().filter(|player| player.eligible_from_question <= index).count().saturating_sub(submissions.len()),
            })),
            Question::FreeText { .. } => Some(json!({
                "kind": "free_text",
                "correct": submissions.iter().filter(|item| item.correct == Some(true)).count(),
                "incorrect": submissions.iter().filter(|item| item.correct == Some(false)).count(),
                "unanswered": self.state.players.values().filter(|player| player.eligible_from_question <= index).count().saturating_sub(submissions.len()),
            })),
        }
    }
}

struct Ranking {
    id: String,
    username: String,
    score: i64,
    rank: usize,
    rank_delta: i64,
}

fn host_question_json(question: &Question) -> Value {
    let mut value = serde_json::to_value(question).expect("questions serialize");
    if let Some(image) = question.image() {
        value["imageUrl"] = json!(format!("/media/{image}"));
        value
            .as_object_mut()
            .expect("question object")
            .remove("image");
    }
    if let Question::MultipleChoice {
        options,
        correct_option_id,
        ..
    } = question
    {
        value["options"] = json!(
            options
                .iter()
                .enumerate()
                .map(|(index, option)| json!({
                    "id": option.id,
                    "text": option.text,
                    "number": index + 1,
                    "shape": ANSWER_STYLES[index].0,
                    "color": ANSWER_STYLES[index].1,
                    "correct": option.id == *correct_option_id,
                }))
                .collect::<Vec<_>>()
        );
    }
    value
}

fn answer_result_json(answer: &SubmittedAnswer, question: Option<&Question>) -> Value {
    match (answer, question) {
        (
            SubmittedAnswer::MultipleChoice { option_id },
            Some(Question::MultipleChoice {
                options,
                correct_option_id,
                ..
            }),
        ) => {
            let selected = options.iter().position(|option| option.id == *option_id);
            let correct = options
                .iter()
                .position(|option| option.id == *correct_option_id);
            json!({
                "kind": "multiple_choice",
                "selected": selected.map(|index| index + 1),
                "correct": correct.map(|index| index + 1),
            })
        }
        (
            SubmittedAnswer::FreeText { text },
            Some(Question::FreeText {
                accepted_answers, ..
            }),
        ) => json!({
            "kind": "free_text",
            "submitted": text,
            "accepted": accepted_answers.first(),
        }),
        _ => Value::Null,
    }
}

pub fn answer_is_correct(question: &Question, answer: &SubmittedAnswer) -> bool {
    match (question, answer) {
        (
            Question::MultipleChoice {
                correct_option_id, ..
            },
            SubmittedAnswer::MultipleChoice { option_id },
        ) => option_id == correct_option_id,
        (
            Question::FreeText {
                accepted_answers, ..
            },
            SubmittedAnswer::FreeText { text },
        ) => {
            let candidate = normalize_answer(text);
            accepted_answers.iter().any(|accepted| {
                let accepted = normalize_answer(accepted);
                let length = accepted.chars().count();
                let threshold = match length {
                    0..=3 => 0,
                    4..=8 => 1,
                    _ => 2,
                };
                damerau_levenshtein(&candidate, &accepted) <= threshold
            })
        }
        _ => false,
    }
}

fn normalize_answer(input: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in input
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
    {
        if character.is_alphanumeric() {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            output.push(character);
            pending_space = false;
        } else {
            pending_space = true;
        }
    }
    output
}

fn damerau_levenshtein(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut matrix = vec![vec![0; right.len() + 1]; left.len() + 1];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, value) in matrix[0].iter_mut().enumerate() {
        *value = j;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                matrix[i][j] = matrix[i][j].min(matrix[i - 2][j - 2] + 1);
            }
        }
    }
    matrix[left.len()][right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::Catalog,
        model::{Choice, Game, GameState},
    };
    use std::path::PathBuf;

    fn free_text(answers: &[&str]) -> Question {
        Question::FreeText {
            id: "q".into(),
            prompt: "Question".into(),
            image: None,
            image_alt: None,
            time_limit_seconds: 20,
            reading_time_seconds: 5,
            double_points: false,
            accepted_answers: answers.iter().map(|answer| (*answer).into()).collect(),
        }
    }

    #[test]
    fn text_matching_normalizes_and_uses_safe_thresholds() {
        let question = free_text(&["Café au lait", "Paris"]);
        assert!(answer_is_correct(
            &question,
            &SubmittedAnswer::FreeText {
                text: " cafe, AU  lait! ".into()
            }
        ));
        assert!(answer_is_correct(
            &question,
            &SubmittedAnswer::FreeText {
                text: "Parsi".into()
            }
        ));
        assert!(!answer_is_correct(
            &free_text(&["cat"]),
            &SubmittedAnswer::FreeText { text: "bat".into() }
        ));
    }

    #[test]
    fn transpositions_count_as_one_edit() {
        assert_eq!(damerau_levenshtein("planet", "palnet"), 1);
    }

    fn quiz(double_points: bool) -> Game {
        Game {
            id: "quiz".into(),
            title: "Quiz".into(),
            description: String::new(),
            questions: vec![Question::MultipleChoice {
                id: "secret-question".into(),
                prompt: "This must never reach players".into(),
                image: None,
                image_alt: None,
                time_limit_seconds: 10,
                reading_time_seconds: 5,
                double_points,
                options: vec![
                    Choice {
                        id: "a".into(),
                        text: "Secret wrong".into(),
                    },
                    Choice {
                        id: "b".into(),
                        text: "Secret right".into(),
                    },
                ],
                correct_option_id: "b".into(),
            }],
        }
    }

    fn engine(double_points: bool) -> Engine {
        Engine::new(
            GameState::default(),
            Catalog {
                games: vec![quiz(double_points)],
                file_path: PathBuf::from("games.json"),
                media_root: PathBuf::from("media"),
            },
            vec!["http://192.168.1.2:8080".into()],
        )
    }

    #[test]
    fn server_clock_scores_and_redacts_player_payloads() {
        let mut engine = engine(true);
        let player_id = engine.join_player("Ada", "token".into()).unwrap();
        engine.select_game("quiz").unwrap();
        engine.start_game(1_000).unwrap();
        assert!(engine.tick(6_000).unwrap());

        let during = engine
            .player_snapshot(&player_id, 6_000)
            .unwrap()
            .to_string();
        assert!(!during.contains("This must never reach players"));
        assert!(!during.contains("Secret right"));
        assert!(during.contains("triangle"));

        engine
            .submit(
                &player_id,
                SubmittedAnswer::MultipleChoice {
                    option_id: "b".into(),
                },
                6_000,
            )
            .unwrap();
        engine.tick(16_000).unwrap();
        let player = engine.state.players.get(&player_id).unwrap();
        assert_eq!(player.score, 2_000);
        assert_eq!(player.correct_count, 1);
        assert!(matches!(engine.state.phase, Phase::Reveal { .. }));
    }

    #[test]
    fn late_joiner_waits_until_the_next_question() {
        let mut engine = engine(false);
        engine.join_player("First", "first".into()).unwrap();
        engine.select_game("quiz").unwrap();
        engine.start_game(0).unwrap();
        engine.tick(5_000).unwrap();
        let late = engine.join_player("Late", "late".into()).unwrap();
        let error = engine
            .submit(
                &late,
                SubmittedAnswer::MultipleChoice {
                    option_id: "b".into(),
                },
                6_000,
            )
            .unwrap_err();
        assert!(error.contains("next question"));
    }

    #[test]
    fn usernames_are_unique_without_case_sensitivity() {
        let mut engine = engine(false);
        engine.join_player("Sam", "one".into()).unwrap();
        assert!(engine.join_player("  sAM  ", "two".into()).is_err());
    }

    #[test]
    fn answering_ends_early_once_every_eligible_player_has_answered() {
        let mut engine = engine(false);
        let player_id = engine.join_player("Ada", "token".into()).unwrap();
        engine.select_game("quiz").unwrap();
        engine.start_game(1_000).unwrap();
        assert!(engine.tick(6_000).unwrap());
        assert!(matches!(engine.state.phase, Phase::Answering { .. }));

        engine
            .submit(
                &player_id,
                SubmittedAnswer::MultipleChoice {
                    option_id: "b".into(),
                },
                6_100,
            )
            .unwrap();

        assert!(matches!(engine.state.phase, Phase::Reveal { .. }));
    }

    #[test]
    fn host_can_force_advance_through_time_based_phases() {
        let mut engine = engine(false);
        engine.join_player("Ada", "token".into()).unwrap();
        engine.select_game("quiz").unwrap();
        engine.start_game(1_000).unwrap();
        assert!(matches!(engine.state.phase, Phase::Reading { .. }));

        engine.advance_host(1_500).unwrap();
        assert!(matches!(engine.state.phase, Phase::Answering { .. }));

        engine.advance_host(1_600).unwrap();
        assert!(matches!(engine.state.phase, Phase::Reveal { .. }));

        engine.advance_host(1_700).unwrap();
        assert!(matches!(engine.state.phase, Phase::FinalLeaderboard));
    }
}
