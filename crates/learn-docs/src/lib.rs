#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Offline command documentation with deterministic fuzzy ranking.

use frizbee::{Config, Matcher, Pattern};
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

    /// Search documents using exact, phrase, and typo-resistant fuzzy matching.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let normalized_query = normalize(query);
        if normalized_query.is_empty() {
            return Vec::new();
        }
        let fuzzy_scores = fuzzy_document_scores(&self.documents, &normalized_query);
        let mut results = self
            .documents
            .iter()
            .enumerate()
            .filter_map(|(document_index, document)| {
                let score = score_document(
                    document,
                    &normalized_query,
                    fuzzy_scores.get(document_index).copied().flatten(),
                )?;
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

fn score_document(document: &Document, query: &str, fuzzy_score: Option<u32>) -> Option<u32> {
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
    if title.split_whitespace().next() == Some(query) {
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
    fuzzy_score.map(|score| 1_000_000u32.saturating_add(score))
}

#[derive(Clone, Debug)]
struct SearchField {
    document_index: usize,
    /// Lower values make a fuzzy match more relevant.
    field_penalty: u32,
    text: String,
}

fn fuzzy_document_scores(documents: &[Document], query: &str) -> Vec<Option<u32>> {
    let fields = documents
        .iter()
        .enumerate()
        .flat_map(|(document_index, document)| {
            let title = std::iter::once(SearchField {
                document_index,
                field_penalty: 32,
                text: normalize(&document.title),
            });
            let aliases = document.aliases.iter().map(move |alias| SearchField {
                document_index,
                field_penalty: 0,
                text: normalize(alias),
            });
            let body = std::iter::once(SearchField {
                document_index,
                field_penalty: 64,
                text: normalize(&document.body),
            });
            title.chain(aliases).chain(body)
        })
        .collect::<Vec<_>>();
    let haystacks = fields
        .iter()
        .map(|field| field.text.as_str())
        .collect::<Vec<_>>();
    let config = Config::default().max_typos(Some(typo_budget(query.chars().count())));
    let mut scores = vec![None; documents.len()];

    if query.split_whitespace().count() == 1 {
        let mut phrase_matcher = Matcher::new(query, &config);
        for matched in phrase_matcher.match_list(&haystacks) {
            update_fuzzy_score(&mut scores, &fields, matched.index, matched.score, 0, 0);
        }
    }

    // A natural-language query should still work when its words are reordered.
    // Frizbee's multi-pattern matcher requires every word to match the same
    // field, while retaining Frizbee's typo tolerance for each word.
    let patterns = Pattern::parse_query(query)
        .into_iter()
        .map(|pattern| {
            let budget = typo_budget(pattern.needle.chars().count());
            pattern.max_typos(Some(budget))
        })
        .collect::<Vec<_>>();
    if patterns.len() > 1 {
        let mut token_matcher = Matcher::from_patterns(&patterns, &config);
        for matched in token_matcher.match_list(&haystacks) {
            let Some(field) = fields.get(matched.index as usize) else {
                continue;
            };
            update_fuzzy_score(
                &mut scores,
                &fields,
                matched.index,
                matched.score,
                256,
                word_shape_penalty(query, &field.text),
            );
        }
    }
    update_document_token_scores(&mut scores, &fields, &haystacks, query);
    scores
}

fn update_document_token_scores(
    scores: &mut [Option<u32>],
    fields: &[SearchField],
    haystacks: &[&str],
    query: &str,
) {
    let tokens = query
        .split_whitespace()
        .filter(|token| token.chars().count() >= 2)
        .collect::<Vec<_>>();
    if tokens.len() < 2 {
        return;
    }

    let mut totals = vec![0u32; scores.len()];
    let mut matched_tokens = vec![0usize; scores.len()];
    for token in tokens {
        let config = Config::default().max_typos(Some(typo_budget(token.chars().count())));
        let mut matcher = Matcher::new(token, &config);
        let mut best_for_document = vec![None; scores.len()];
        for matched in matcher.match_list(haystacks) {
            let Some(field) = fields.get(matched.index as usize) else {
                continue;
            };
            let score = u32::from(u16::MAX)
                .saturating_sub(u32::from(matched.score))
                .saturating_add(field.field_penalty)
                .saturating_add(word_shape_penalty(token, &field.text));
            let document_score = &mut best_for_document[field.document_index];
            if document_score.is_none_or(|current| score < current) {
                *document_score = Some(score);
            }
        }
        for (document_index, best_score) in best_for_document.into_iter().enumerate() {
            if let Some(best_score) = best_score {
                totals[document_index] = totals[document_index].saturating_add(best_score);
                matched_tokens[document_index] += 1;
            }
        }
    }

    let required_tokens = query
        .split_whitespace()
        .filter(|token| token.chars().count() >= 2)
        .count();
    for (document_index, total) in totals.into_iter().enumerate() {
        if matched_tokens[document_index] != required_tokens {
            continue;
        }
        let average = total / u32::try_from(required_tokens).unwrap_or(1);
        let score = 1_000_000u32.saturating_add(average).saturating_add(128);
        if scores[document_index].is_none_or(|current| score < current) {
            scores[document_index] = Some(score);
        }
    }
}

fn update_fuzzy_score(
    scores: &mut [Option<u32>],
    fields: &[SearchField],
    field_index: u32,
    match_score: u16,
    match_penalty: u32,
    shape_penalty: u32,
) {
    let Some(field) = fields.get(field_index as usize) else {
        return;
    };
    let score = u32::from(u16::MAX)
        .saturating_sub(u32::from(match_score))
        .saturating_add(field.field_penalty)
        .saturating_add(match_penalty)
        .saturating_add(shape_penalty);
    let document_score = &mut scores[field.document_index];
    if document_score.is_none_or(|current| score < current) {
        *document_score = Some(score);
    }
}

fn word_shape_penalty(query: &str, candidate: &str) -> u32 {
    let candidate_words = candidate.split_whitespace().collect::<Vec<_>>();
    query
        .split_whitespace()
        .map(|query_word| {
            candidate_words
                .iter()
                .map(|candidate_word| {
                    query_word
                        .chars()
                        .count()
                        .abs_diff(candidate_word.chars().count())
                })
                .min()
                .unwrap_or(8)
        })
        .sum::<usize>()
        .saturating_mul(8)
        .try_into()
        .unwrap_or(u32::MAX)
}

fn typo_budget(character_count: usize) -> u16 {
    match character_count {
        0..=2 => 0,
        3..=4 => 1,
        _ => u16::try_from((character_count / 4).clamp(1, 6)).unwrap_or(6),
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else if matches!(character, '>' | '|' | '*' | '<') {
                character
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
#[allow(clippy::too_many_lines)]
pub fn starter_index() -> DocsIndex {
    let mut documents = vec![
        Document {
            id: "command.pwd".to_owned(),
            title: "pwd — print the working directory".to_owned(),
            body: "Use pwd to see the full path of the folder your terminal is currently using. Example: pwd".to_owned(),
            aliases: vec![
                "where am i".to_owned(),
                "current directory".to_owned(),
                "current folder".to_owned(),
                "show current location".to_owned(),
            ],
        },
        Document {
            id: "command.cd".to_owned(),
            title: "cd — change folders".to_owned(),
            body: "Use cd to go into another folder. Example: cd ~/project. A path of ~ means your home folder.".to_owned(),
            aliases: vec![
                "change folder".to_owned(),
                "change directory".to_owned(),
                "go to a folder".to_owned(),
                "move into a folder".to_owned(),
            ],
        },
        Document {
            id: "command.ls".to_owned(),
            title: "ls — list files and folders".to_owned(),
            body: "Use ls to see the files and folders in the current location. Example: ls or ls ~/project.".to_owned(),
            aliases: vec![
                "list files".to_owned(),
                "show files".to_owned(),
                "see folder contents".to_owned(),
                "what is in this folder".to_owned(),
            ],
        },
        Document {
            id: "command.mkdir".to_owned(),
            title: "mkdir — create a folder".to_owned(),
            body: "Use mkdir to create a new folder. Example: mkdir ~/project creates a folder named project.".to_owned(),
            aliases: vec![
                "create a folder".to_owned(),
                "make a folder".to_owned(),
                "create a directory".to_owned(),
                "make a directory".to_owned(),
                "new folder".to_owned(),
            ],
        },
        Document {
            id: "command.touch".to_owned(),
            title: "touch — create an empty file".to_owned(),
            body: "Use touch to create a new empty file. Example: touch ~/project/notes.txt creates notes.txt inside project.".to_owned(),
            aliases: vec![
                "create a file".to_owned(),
                "create a text file".to_owned(),
                "make a file".to_owned(),
                "new file".to_owned(),
                "empty file".to_owned(),
            ],
        },
        Document {
            id: "command.cat".to_owned(),
            title: "cat — show a file".to_owned(),
            body: "Use cat to read a file and show its contents in the terminal. Example: cat ~/welcome.txt.".to_owned(),
            aliases: vec![
                "read a file".to_owned(),
                "show file contents".to_owned(),
                "print a file".to_owned(),
                "open a text file".to_owned(),
            ],
        },
        Document {
            id: "command.less".to_owned(),
            title: "less — read a file page by page".to_owned(),
            body: "Use less to read a long file one screen at a time. In this lesson shell, cat and less both display file contents.".to_owned(),
            aliases: vec![
                "read page by page".to_owned(),
                "scroll through a file".to_owned(),
                "read a long file".to_owned(),
            ],
        },
        Document {
            id: "command.mv".to_owned(),
            title: "mv — move or rename a file".to_owned(),
            body: "Use mv to move a file into another folder or give it a new name. Example: mv note.txt archive/.".to_owned(),
            aliases: vec![
                "move a file".to_owned(),
                "rename a file".to_owned(),
                "put a file somewhere else".to_owned(),
            ],
        },
        Document {
            id: "command.cp".to_owned(),
            title: "cp — copy files".to_owned(),
            body: "Use cp to make a copy of a file. Example: cp note.txt backup.txt. A filename pattern can copy several files.".to_owned(),
            aliases: vec![
                "copy a file".to_owned(),
                "duplicate a file".to_owned(),
                "copy folder".to_owned(),
            ],
        },
        Document {
            id: "command.rm".to_owned(),
            title: "rm — remove a file".to_owned(),
            body: "Use rm to remove a file from the virtual lesson home. Example: rm old.txt. Check the name carefully before removing files.".to_owned(),
            aliases: vec![
                "delete a file".to_owned(),
                "remove a file".to_owned(),
                "get rid of a file".to_owned(),
            ],
        },
        Document {
            id: "command.find".to_owned(),
            title: "find — search for files".to_owned(),
            body: "Use find to look through a folder and its subfolders for a file. Example: find ~/archive -name treasure.txt.".to_owned(),
            aliases: vec![
                "find file by name".to_owned(),
                "locate a file".to_owned(),
                "search for a file".to_owned(),
                "look through folders".to_owned(),
            ],
        },
        Document {
            id: "command.grep".to_owned(),
            title: "grep — search text".to_owned(),
            body: "Use grep to find and show lines containing a word or phrase. Example: grep URGENT service.log.".to_owned(),
            aliases: vec![
                "search text in file".to_owned(),
                "find text".to_owned(),
                "find a line".to_owned(),
                "filter lines".to_owned(),
            ],
        },
        Document {
            id: "command.head".to_owned(),
            title: "head — show the first lines".to_owned(),
            body: "Use head to show the beginning of a file or the beginning of command output.".to_owned(),
            aliases: vec![
                "show first lines".to_owned(),
                "first lines of a file".to_owned(),
                "file beginning".to_owned(),
            ],
        },
        Document {
            id: "command.wc".to_owned(),
            title: "wc — count input".to_owned(),
            body: "Use wc to count the bytes received from a file or another command. It is useful for measuring output.".to_owned(),
            aliases: vec![
                "count file size".to_owned(),
                "count bytes".to_owned(),
                "count words".to_owned(),
            ],
        },
        Document {
            id: "command.echo".to_owned(),
            title: "echo — print words".to_owned(),
            body: "Use echo to print words or a short message in the terminal. Example: echo hello.".to_owned(),
            aliases: vec!["print words".to_owned(), "show a message".to_owned()],
        },
        Document {
            id: "command.printf".to_owned(),
            title: "printf — print exact text".to_owned(),
            body: "Use printf when you want to print exact text. It is useful with > when saving text into a file.".to_owned(),
            aliases: vec![
                "print exact text".to_owned(),
                "write exact text".to_owned(),
                "output text".to_owned(),
            ],
        },
        Document {
            id: "command.clear".to_owned(),
            title: "clear — clear the terminal".to_owned(),
            body: "Use clear to remove old output from the visible terminal. It does not delete files or reset the lesson.".to_owned(),
            aliases: vec![
                "clear terminal".to_owned(),
                "clean screen".to_owned(),
                "remove terminal output".to_owned(),
            ],
        },
        Document {
            id: "command.exit".to_owned(),
            title: "exit — leave the shell".to_owned(),
            body: "Use exit to close the current lesson terminal. The command is always available.".to_owned(),
            aliases: vec![
                "leave terminal".to_owned(),
                "close shell".to_owned(),
                "quit terminal".to_owned(),
            ],
        },
        Document {
            id: "command.help".to_owned(),
            title: "help — list available commands".to_owned(),
            body: "Use help for a short list of commands supported by the current lesson. The docs pane explains what each command does.".to_owned(),
            aliases: vec![
                "available commands".to_owned(),
                "what commands can i use".to_owned(),
                "command list".to_owned(),
            ],
        },
        Document {
            id: "shell.redirect".to_owned(),
            title: "> — save command output to a file".to_owned(),
            body: "Use > to send a command's output into a file instead of only showing it. Example: printf saved > result.txt.".to_owned(),
            aliases: vec![
                "save output".to_owned(),
                "write command output".to_owned(),
                "redirect output".to_owned(),
                "put output in a file".to_owned(),
            ],
        },
        Document {
            id: "shell.pipe".to_owned(),
            title: "| — pass output to another command".to_owned(),
            body: "Use | to send the output of one command into the next command. Example: cat lines.txt | grep Linux.".to_owned(),
            aliases: vec![
                "connect commands".to_owned(),
                "pass output".to_owned(),
                "combine commands".to_owned(),
                "pipe commands".to_owned(),
            ],
        },
        Document {
            id: "shell.glob".to_owned(),
            title: "* — match many filenames".to_owned(),
            body: "Use * as a filename wildcard. For example, *.log means every filename ending in .log.".to_owned(),
            aliases: vec![
                "all log files".to_owned(),
                "filename wildcard".to_owned(),
                "match many files".to_owned(),
                "copy all files".to_owned(),
            ],
        },
        Document {
            id: "command.true".to_owned(),
            title: "true — do nothing successfully".to_owned(),
            body: "Use true when a command that succeeds without producing output is needed. It is mostly useful in shell experiments.".to_owned(),
            aliases: vec!["do nothing".to_owned(), "successful no-op".to_owned()],
        },
    ];
    documents.extend([
        manual_document(
            "man",
            "read a local command manual",
            "manual reference instructions",
        ),
        manual_document(
            "chmod",
            "change file permissions",
            "permissions octal owner group world",
        ),
        manual_document("stat", "show file mode", "inspect permissions mode"),
        manual_document("tail", "show the last lines", "end of file final lines"),
        manual_document("sort", "sort lines", "alphabetical order"),
        manual_document(
            "uniq",
            "remove adjacent duplicate lines",
            "deduplicate repeated lines",
        ),
    ]);
    for document in &mut documents {
        if let Some(command) = document.id.strip_prefix("command.")
            && let Some(details) = manual_details(command)
        {
            document.body.push_str("\n\n");
            document.body.push_str(details);
        }
    }
    DocsIndex::new(documents)
}

/// Return a command's offline manual from the same content as the docs pane.
pub fn command_manual(command: &str) -> Option<String> {
    let id = format!("command.{command}");
    starter_index()
        .documents
        .into_iter()
        .find(|document| document.id == id)
        .map(|document| format!("{}\n\n{}\n", document.title, document.body))
}

fn manual_document(command: &str, summary: &str, aliases: &str) -> Document {
    Document {
        id: format!("command.{command}"),
        title: format!("{command} — {summary}"),
        body: format!(
            "Use {command} in the lesson shell. Run man {command} or {command} --help for supported syntax."
        ),
        aliases: aliases.split_whitespace().map(str::to_owned).collect(),
    }
}

fn manual_details(command: &str) -> Option<&'static str> {
    Some(match command {
        "pwd" => "USAGE: pwd\nPrint the current directory.",
        "cd" => "USAGE: cd [DIR]\nChange directory. ~ means your home.",
        "ls" => "USAGE: ls [DIR]\nList immediate children.",
        "mkdir" => "USAGE: mkdir [-p] DIR...\nCreate folders; -p also creates missing parents.",
        "touch" => "USAGE: touch FILE...\nCreate empty files without replacing existing contents.",
        "cat" => "USAGE: cat [FILE...]\nPrint files or piped input.",
        "less" => "USAGE: less FILE\nDisplay a file in this lesson shell.",
        "cp" => "USAGE: cp SOURCE... DEST\nCopy files; use * to select several names.",
        "mv" => "USAGE: mv SOURCE DEST\nMove or rename one file.",
        "rm" => "USAGE: rm FILE...\nRemove files in the virtual home; folders are not removed.",
        "find" => "USAGE: find DIR [-name NAME]\nSearch a directory tree for a filename.",
        "grep" => "USAGE: grep PATTERN [FILE]\nPrint matching lines from a file or pipe.",
        "head" => "USAGE: head [FILE]\nPrint the first ten lines.",
        "tail" => "USAGE: tail [FILE]\nPrint the last ten lines.",
        "sort" => "USAGE: sort [FILE]\nSort lines alphabetically; also accepts piped input.",
        "uniq" => {
            "USAGE: uniq [FILE]\nRemove adjacent duplicate lines; sort first for global deduplication."
        }
        "wc" => "USAGE: wc\nCount input bytes from a pipe in this lesson shell.",
        "echo" => "USAGE: echo WORD...\nPrint words followed by a newline.",
        "printf" => "USAGE: printf TEXT\nPrint text without adding a newline.",
        "chmod" => {
            "USAGE: chmod MODE FILE...\nSet file permissions. MODE can be octal (600, 644, 755) or symbolic (u+x, go-r, u=rw,go=). r=read, w=write, x=execute."
        }
        "stat" => "USAGE: stat FILE\nShow the file's octal permission mode.",
        "man" => "USAGE: man COMMAND\nRead the local manual for a supported command.",
        "help" => "USAGE: help [COMMAND]\nList commands or read one command's help.",
        "clear" => "USAGE: clear\nClear the terminal display.",
        "exit" => "USAGE: exit\nClose the current lesson shell.",
        "true" => "USAGE: true\nSucceed without output.",
        _ => return None,
    })
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
        assert_eq!(index.search("move a file", 1)[0].document.id, "command.mv");
        assert_eq!(
            index.search("create a file", 1)[0].document.id,
            "command.touch"
        );
        assert_eq!(
            index.search("create a folder", 1)[0].document.id,
            "command.mkdir"
        );
        assert_eq!(
            index.search("show file contents", 1)[0].document.id,
            "command.cat"
        );
        assert_eq!(
            index.search("save output", 1)[0].document.id,
            "shell.redirect"
        );
        assert_eq!(index.search(">", 1)[0].document.id, "shell.redirect");
        assert_eq!(index.search("|", 1)[0].document.id, "shell.pipe");
        assert_eq!(index.search("*", 1)[0].document.id, "shell.glob");
    }

    #[test]
    fn fuzzy_search_handles_typos_and_reordered_words() {
        let index = starter_index();
        let cases = [
            ("creat a fle", "command.touch"),
            ("mk a foler", "command.mkdir"),
            ("read contnts", "command.cat"),
            ("text search fiel", "command.grep"),
            ("locate treasur", "command.find"),
            ("save commnd otput", "shell.redirect"),
        ];

        for (query, expected_id) in cases {
            assert_eq!(
                index.search(query, 1)[0].document.id,
                expected_id,
                "query {query:?}"
            );
        }
    }
}
