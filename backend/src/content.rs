use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::model::{Game, GamesFile, Question};

#[derive(Clone)]
pub struct Catalog {
    pub games: Vec<Game>,
    pub file_path: PathBuf,
    pub media_root: PathBuf,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("could not read game file {}", path.display()))?;
        let parsed: GamesFile = serde_json::from_str(&raw)
            .with_context(|| format!("invalid JSON in {}", path.display()))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let media_root = parent.join("media");
        validate_games(&parsed.games, &media_root)?;
        Ok(Self {
            games: parsed.games,
            file_path: path.to_path_buf(),
            media_root,
        })
    }

    pub fn reload(&self) -> Result<Self> {
        Self::load(&self.file_path)
    }

    pub fn game(&self, id: &str) -> Option<&Game> {
        self.games.iter().find(|game| game.id == id)
    }
}

fn validate_games(games: &[Game], media_root: &Path) -> Result<()> {
    if games.is_empty() {
        bail!("games must contain at least one game");
    }
    let mut game_ids = HashSet::new();
    for game in games {
        if game.id.trim().is_empty() || !game_ids.insert(game.id.as_str()) {
            bail!(
                "game IDs must be non-empty and unique (problem: {:?})",
                game.id
            );
        }
        if game.title.trim().is_empty() || game.questions.is_empty() {
            bail!("game {} needs a title and at least one question", game.id);
        }
        let mut question_ids = HashSet::new();
        for question in &game.questions {
            if question.id().trim().is_empty() || !question_ids.insert(question.id()) {
                bail!(
                    "question IDs in game {} must be non-empty and unique",
                    game.id
                );
            }
            if !(5..=300).contains(&question.time_limit_seconds()) {
                bail!(
                    "question {} timeLimitSeconds must be between 5 and 300",
                    question.id()
                );
            }
            if question.reading_time_seconds() > 60 {
                bail!(
                    "question {} readingTimeSeconds must be between 0 and 60",
                    question.id()
                );
            }
            match question {
                Question::MultipleChoice {
                    prompt,
                    image,
                    image_alt,
                    options,
                    correct_option_id,
                    ..
                } => {
                    validate_common(question.id(), prompt, image, image_alt, media_root)?;
                    if !(2..=4).contains(&options.len()) {
                        bail!("question {} must have 2 to 4 options", question.id());
                    }
                    let mut ids = HashSet::new();
                    for option in options {
                        if option.id.trim().is_empty()
                            || option.text.trim().is_empty()
                            || !ids.insert(option.id.as_str())
                        {
                            bail!(
                                "question {} has an empty or duplicate option",
                                question.id()
                            );
                        }
                    }
                    if !ids.contains(correct_option_id.as_str()) {
                        bail!(
                            "question {} correctOptionId does not name an option",
                            question.id()
                        );
                    }
                }
                Question::FreeText {
                    prompt,
                    image,
                    image_alt,
                    accepted_answers,
                    ..
                } => {
                    validate_common(question.id(), prompt, image, image_alt, media_root)?;
                    if accepted_answers.is_empty()
                        || accepted_answers
                            .iter()
                            .any(|answer| answer.trim().is_empty())
                    {
                        bail!("question {} needs non-empty acceptedAnswers", question.id());
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_common(
    id: &str,
    prompt: &str,
    image: &Option<String>,
    image_alt: &Option<String>,
    media_root: &Path,
) -> Result<()> {
    if prompt.trim().is_empty() {
        bail!("question {id} needs a prompt");
    }
    if let Some(image) = image {
        if image_alt.as_deref().is_none_or(|alt| alt.trim().is_empty()) {
            bail!("question {id} must provide imageAlt when image is present");
        }
        let relative = Path::new(image);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            bail!("question {id} image path must stay inside the media directory");
        }
        let extension = relative
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
            bail!("question {id} image must be PNG, JPEG, WebP, or GIF");
        }
        let full_path = media_root.join(relative);
        let canonical_root = fs::canonicalize(media_root)
            .with_context(|| format!("media directory {} was not found", media_root.display()))?;
        let canonical_path = fs::canonicalize(&full_path).with_context(|| {
            format!("question {id} image {} was not found", full_path.display())
        })?;
        if !canonical_path.starts_with(&canonical_root) {
            bail!("question {id} image path must stay inside the media directory");
        }
        let metadata = fs::metadata(&full_path).with_context(|| {
            format!("question {id} image {} was not found", full_path.display())
        })?;
        if !metadata.is_file() || metadata.len() > 10 * 1024 * 1024 {
            bail!("question {id} image must be a file no larger than 10 MiB");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../content/games.json");
        let catalog = Catalog::load(&path).expect("bundled games should load");
        assert_eq!(catalog.games.len(), 2);
        assert!(catalog.game("welcome-to-hoot").is_some());
    }
}
