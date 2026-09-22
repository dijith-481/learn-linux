#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Offline command documentation with deterministic fuzzy ranking.

use serde::{Deserialize, Serialize};

/// One searchable documentation chunk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Stable ID.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Searchable body.
    pub body: String,
    /// Command/topic aliases.
    pub aliases: Vec<String>,
}

/// Search result with a stable score.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    /// Matched document.
    pub document: Document,
    /// Lower scores are better.
    pub score: u32,
    /// Short context.
    pub snippet: String,
}

/// In-memory local documentation index.
#[derive(Clone, Debug, Default)]
pub struct DocsIndex {
    documents: Vec<Document>,
}

impl DocsIndex {
    /// Build an index from documents.
    pub fn new(documents: Vec<Document>) -> Self {
        Self { documents }
    }

    /// Search documents using token-aware subsequence matching.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let normalized_query = normalize(query);
        if normalized_query.is_empty() {
            return Vec::new();
        }
        let mut results = self
            .documents
            .iter()
            .filter_map(|document| {
                let score = score_document(document, &normalized_query)?;
                Some(SearchResult {
                    snippet: snippet(&document.body),
                    document: document.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            left.score
                .cmp(&right.score)
                .then_with(|| left.document.id.cmp(&right.document.id))
        });
        results.truncate(limit);
        results
    }

    /// Number of indexed documents.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Whether the index has no documents.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

fn score_document(document: &Document, query: &str) -> Option<u32> {
    let title = normalize(&document.title);
    let body = normalize(&document.body);
    let aliases = document
        .aliases
        .iter()
        .map(|value| normalize(value))
        .collect::<Vec<_>>();
    if title == query {
        return Some(0);
    }
    if aliases.iter().any(|alias| alias == query) {
        return Some(1);
    }
    if title.contains(query) {
        let title_length = u32::try_from(title.len()).unwrap_or(u32::MAX);
        let query_length = u32::try_from(query.len()).unwrap_or(u32::MAX);
        return Some(5 + title_length.saturating_sub(query_length));
    }
    if aliases.iter().any(|alias| alias.contains(query)) {
        return Some(10);
    }
    if body.contains(query) {
        return Some(20);
    }
    let candidates = std::iter::once(title)
        .chain(aliases)
        .chain(std::iter::once(body));
    let distance = candidates
        .map(|candidate| edit_distance(query, &candidate))
        .min()?;
    let query_length = u32::try_from(query.chars().count()).unwrap_or(u32::MAX);
    let threshold = 3.max(query_length / 4);
    (distance <= threshold).then_some(40 + distance)
}

fn edit_distance(left: &str, right: &str) -> u32 {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len())
        .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
        .collect::<Vec<_>>();
    for (left_index, left_character) in left.chars().enumerate() {
        let left_index = u32::try_from(left_index).unwrap_or(u32::MAX);
        let mut current = vec![left_index.saturating_add(1)];
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + u32::from(left_character != *right_character);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn snippet(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(140).collect()
}

/// The built-in starter documentation index.
pub fn starter_index() -> DocsIndex {
    DocsIndex::new(vec![
        Document {
            id: "command.pwd".to_owned(),
            title: "pwd — print the working directory".to_owned(),
            body: "Use pwd to see the absolute path of the directory your shell is currently using.".to_owned(),
            aliases: vec!["where am i".to_owned(), "current directory".to_owned()],
        },
        Document {
            id: "command.find".to_owned(),
            title: "find — search for files".to_owned(),
            body: "Use find to walk a directory tree and select files by name, type, or other properties.".to_owned(),
            aliases: vec!["find file by name".to_owned(), "locate a file".to_owned()],
        },
        Document {
            id: "command.grep".to_owned(),
            title: "grep — search text".to_owned(),
            body: "Use grep to print lines matching a pattern in input or files.".to_owned(),
            aliases: vec!["search text in file".to_owned(), "filter lines".to_owned()],
        },
        Document {
            id: "command.chmod".to_owned(),
            title: "chmod — change permissions".to_owned(),
            body: "Use chmod to change read, write, and execute mode bits on a file.".to_owned(),
            aliases: vec!["change permissions".to_owned(), "file mode".to_owned()],
        },
        Document {
            id: "command.kill".to_owned(),
            title: "kill — send a signal to a process".to_owned(),
            body: "Use kill to send a selected signal to a process identified by PID.".to_owned(),
            aliases: vec!["stop process".to_owned(), "terminate program".to_owned()],
        },
        Document {
            id: "command.cp".to_owned(),
            title: "cp — copy files and directories".to_owned(),
            body: "Use cp to copy a file or, with a recursive option, a directory tree.".to_owned(),
            aliases: vec!["copy folder".to_owned(), "duplicate file".to_owned()],
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typo_tolerant_search_returns_expected_command() {
        let index = starter_index();
        assert_eq!(
            index.search("serch text in fiel", 1)[0].document.id,
            "command.grep"
        );
        assert_eq!(index.search("where am i", 1)[0].document.id, "command.pwd");
        assert_eq!(
            index.search("stop proces", 1)[0].document.id,
            "command.kill"
        );
    }
}
