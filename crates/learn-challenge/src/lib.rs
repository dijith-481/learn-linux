#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! KDL authoring, semantic decoding, and deterministic challenge validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use kdl::{KdlDocument, KdlNode, KdlValue};
use learn_core::{
    ChallengeId, EnvironmentSpec, NetworkPolicy, SandboxPath, SetupAction, ShellKind, StateMode,
    ToolId, ToolRestrictionStrength, VerifierExpr, VerifierPredicate,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A human-facing challenge difficulty.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Difficulty {
    /// Introductory material.
    Beginner,
    /// Requires combining known tools.
    Intermediate,
    /// Advanced material.
    Advanced,
}

impl Difficulty {
    fn parse(value: &str) -> Result<Self, CompileError> {
        match value {
            "beginner" => Ok(Self::Beginner),
            "intermediate" => Ok(Self::Intermediate),
            "advanced" => Ok(Self::Advanced),
            other => Err(CompileError::InvalidValue(format!(
                "unknown difficulty {other}"
            ))),
        }
    }
}

/// Challenge display metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChallengeMeta {
    /// Title shown to learners.
    pub title: String,
    /// One-sentence summary.
    pub summary: String,
    /// Difficulty.
    pub difficulty: Difficulty,
    /// Expected minutes.
    pub estimated_minutes: u16,
}

/// Skills attached to a challenge.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChallengeSkills {
    /// Skills expected before attempting it.
    pub requires: Vec<String>,
    /// Skills introduced by it.
    pub teaches: Vec<String>,
    /// Skills practiced again.
    pub reinforces: Vec<String>,
}

/// A learner-facing objective.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    /// Objective text.
    pub text: String,
}

/// A progressively revealed hint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    /// Hint level.
    pub level: u8,
    /// Hint text.
    pub text: String,
    /// Optional documentation ID.
    pub doc: Option<String>,
}

/// Canonical solution explanation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalSolution {
    /// Concept explained after completion.
    pub concept: String,
    /// Suggested commands in order.
    pub commands: Vec<String>,
    /// Explanation bullets.
    pub why: Vec<String>,
}

/// Fully decoded challenge before runtime-specific planning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChallengeSpec {
    /// Stable identity.
    pub id: ChallengeId,
    /// Presentation metadata.
    pub meta: ChallengeMeta,
    /// Skill relationships.
    pub skills: ChallengeSkills,
    /// Learner objective.
    pub objective: Objective,
    /// Runtime environment.
    pub environment: EnvironmentSpec,
    /// Deterministic setup.
    pub setup: Vec<SetupAction>,
    /// Result-state verifier.
    pub verifier: VerifierExpr,
    /// Hints.
    pub hints: Vec<Hint>,
    /// Canonical solution.
    pub solution: CanonicalSolution,
    /// Referenced documentation IDs.
    pub docs: Vec<String>,
    /// Optional visual name.
    pub visual: Option<String>,
}

/// A compiled challenge with source provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledChallenge {
    /// Validated challenge specification.
    pub spec: ChallengeSpec,
    /// Source file used to create it.
    pub source: String,
}

/// A content pack produced by compiling a directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentPack {
    /// Compiled challenges in stable ID order.
    pub challenges: Vec<CompiledChallenge>,
}

impl ContentPack {
    /// Validate unique challenge IDs and return the number of challenges.
    pub fn validate(&self) -> Result<usize, CompileError> {
        let mut seen = BTreeSet::new();
        for challenge in &self.challenges {
            if !seen.insert(challenge.spec.id.clone()) {
                return Err(CompileError::DuplicateChallenge(
                    challenge.spec.id.to_string(),
                ));
            }
            validate_spec(&challenge.spec)?;
        }
        Ok(self.challenges.len())
    }

    /// Find a challenge by ID.
    pub fn get(&self, id: &str) -> Option<&CompiledChallenge> {
        self.challenges
            .iter()
            .find(|item| item.spec.id.as_str() == id)
    }
}

/// Compile a KDL document containing one or more challenge nodes.
pub fn compile_source(
    source: &str,
    source_name: impl Into<String>,
) -> Result<Vec<CompiledChallenge>, CompileError> {
    let source_name = source_name.into();
    let document: KdlDocument = source
        .parse()
        .map_err(|error| CompileError::Parse(format!("{source_name}: {error}")))?;
    document
        .nodes()
        .iter()
        .filter(|node| node.name().value() == "challenge")
        .map(|node| compile_node(node, &source_name))
        .collect()
}

/// Compile every KDL file in a directory.
pub fn compile_directory(directory: impl AsRef<Path>) -> Result<ContentPack, CompileError> {
    let directory = directory.as_ref();
    let mut paths = fs::read_dir(directory)
        .map_err(|error| CompileError::Io(format!("{}: {error}", directory.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "kdl"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut challenges = Vec::new();
    for path in paths {
        let source = fs::read_to_string(&path)
            .map_err(|error| CompileError::Io(format!("{}: {error}", path.display())))?;
        challenges.extend(compile_source(&source, path.display().to_string())?);
    }
    challenges.sort_by(|left, right| left.spec.id.cmp(&right.spec.id));
    let pack = ContentPack { challenges };
    pack.validate()?;
    Ok(pack)
}

fn compile_node(node: &KdlNode, source: &str) -> Result<CompiledChallenge, CompileError> {
    let id = ChallengeId::new(string_argument(node, 0, "challenge ID")?)?;
    let children = child_map(node);
    let difficulty =
        optional_string(&children, "difficulty").unwrap_or_else(|| "beginner".to_owned());
    let meta = ChallengeMeta {
        title: required_string(&children, "title")?,
        summary: required_string(&children, "summary")?,
        difficulty: Difficulty::parse(&difficulty)?,
        estimated_minutes: optional_integer(&children, "estimated-minutes")
            .unwrap_or(5)
            .try_into()
            .map_err(|_| {
                CompileError::InvalidValue("estimated-minutes must fit in u16".to_owned())
            })?,
    };
    let skills = parse_skills(children.get("skills"))?;
    let objective_node = children
        .get("objective")
        .ok_or(CompileError::MissingNode("objective"))?;
    let objective_children = child_map(objective_node);
    let objective = Objective {
        text: child_required_string(objective_children.get("text"), "text")?,
    };
    let environment = parse_environment(&children)?;
    let setup = parse_setup(children.get("setup"))?;
    let verifier = parse_verifier(children.get("verify"))?;
    let hints = parse_hints(children.get("hints").or_else(|| children.get("hint")))?;
    let solution = parse_solution(children.get("solution"))?;
    let docs = children
        .get("docs")
        .map(|node| {
            child_nodes(Some(node))
                .iter()
                .filter(|child| child.name().value() == "doc")
                .map(|child| string_argument(child, 0, "doc ID"))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let visual = children
        .get("visual")
        .and_then(|node| string_argument(node, 0, "visual name").ok());
    let spec = ChallengeSpec {
        id,
        meta,
        skills,
        objective,
        environment,
        setup,
        verifier,
        hints,
        solution,
        docs,
        visual,
    };
    validate_spec(&spec)?;
    Ok(CompiledChallenge {
        spec,
        source: source.to_owned(),
    })
}

fn parse_skills(node: Option<&KdlNode>) -> Result<ChallengeSkills, CompileError> {
    let Some(node) = node else {
        return Ok(ChallengeSkills::default());
    };
    let mut skills = ChallengeSkills::default();
    for child in child_nodes(Some(node)) {
        let value = string_argument(child, 0, "skill ID")?;
        match child.name().value() {
            "requires" => skills.requires.push(value),
            "teaches" => skills.teaches.push(value),
            "reinforces" => skills.reinforces.push(value),
            other => return Err(CompileError::UnknownNode(other.to_owned())),
        }
    }
    Ok(skills)
}

fn parse_environment(
    children: &BTreeMap<String, KdlNode>,
) -> Result<EnvironmentSpec, CompileError> {
    let state = match optional_string(children, "state")
        .as_deref()
        .unwrap_or("ephemeral")
    {
        "ephemeral" => StateMode::Ephemeral,
        "persistent-home" => StateMode::PersistentHome,
        "persistent-world" => StateMode::PersistentWorld,
        value => return Err(CompileError::InvalidValue(format!("unknown state {value}"))),
    };
    let shell = match optional_string(children, "shell")
        .as_deref()
        .unwrap_or("bash")
    {
        "bash" => ShellKind::Bash,
        value => {
            return Err(CompileError::InvalidValue(format!(
                "unsupported shell {value}"
            )));
        }
    };
    let capabilities = children.get("capabilities");
    let capability_children = capabilities.map(child_map).unwrap_or_default();
    let network = match capability_children
        .get("network")
        .and_then(|node| string_argument(node, 0, "network policy").ok())
        .as_deref()
        .unwrap_or("none")
    {
        "none" => NetworkPolicy::None,
        "loopback" => NetworkPolicy::Loopback,
        "host" => NetworkPolicy::HostNetwork,
        value => {
            return Err(CompileError::InvalidValue(format!(
                "unknown network policy {value}"
            )));
        }
    };
    let mut allowed_tools = BTreeSet::new();
    let Some(tools) = capability_children.get("tools") else {
        return Ok(EnvironmentSpec {
            state,
            shell,
            network,
            allowed_tools,
            restriction: ToolRestrictionStrength::Guidance,
        });
    };
    let mut has_tools = false;
    for child in child_nodes(Some(tools)) {
        if child.name().value() == "allow" {
            allowed_tools.insert(ToolId::new(string_argument(child, 0, "tool ID")?)?);
            has_tools = true;
        }
    }
    Ok(EnvironmentSpec {
        state,
        shell,
        network,
        allowed_tools,
        restriction: if has_tools {
            ToolRestrictionStrength::ExternalExecutableVisibility
        } else {
            ToolRestrictionStrength::Guidance
        },
    })
}

fn parse_setup(node: Option<&KdlNode>) -> Result<Vec<SetupAction>, CompileError> {
    let Some(node) = node else {
        return Ok(Vec::new());
    };
    child_nodes(Some(node))
        .iter()
        .map(|child| match child.name().value() {
            "mkdir" => Ok(SetupAction::CreateDir {
                path: SandboxPath::parse(string_argument(child, 0, "directory path")?)?,
            }),
            "write" => {
                let path = SandboxPath::parse(string_argument(child, 0, "file path")?)?;
                let text = child_required_string(child_map(child).get("text"), "text")?;
                Ok(SetupAction::WriteFile {
                    path,
                    contents: text.into_bytes(),
                })
            }
            "mode" => {
                let path = SandboxPath::parse(string_argument(child, 0, "file path")?)?;
                let mode = integer_argument(child, 1, "mode")?;
                let mode = u32::try_from(mode)
                    .map_err(|_| CompileError::InvalidValue("mode must be positive".to_owned()))?;
                Ok(SetupAction::SetMode { path, mode })
            }
            other => Err(CompileError::UnknownNode(other.to_owned())),
        })
        .collect()
}

fn parse_verifier(node: Option<&KdlNode>) -> Result<VerifierExpr, CompileError> {
    let Some(node) = node else {
        return Err(CompileError::MissingNode("verify"));
    };
    let root = child_nodes(Some(node))
        .first()
        .copied()
        .ok_or(CompileError::MissingNode("verify predicate"))?;
    parse_verifier_node(root)
}

fn parse_verifier_node(node: &KdlNode) -> Result<VerifierExpr, CompileError> {
    match node.name().value() {
        "all" => Ok(VerifierExpr::All(
            child_nodes(Some(node))
                .iter()
                .map(|child| parse_verifier_node(child))
                .collect::<Result<_, _>>()?,
        )),
        "any" => Ok(VerifierExpr::Any(
            child_nodes(Some(node))
                .iter()
                .map(|child| parse_verifier_node(child))
                .collect::<Result<_, _>>()?,
        )),
        "not" => Ok(VerifierExpr::Not(Box::new(parse_verifier_node(
            child_nodes(Some(node))
                .first()
                .copied()
                .ok_or(CompileError::MissingNode("not predicate"))?,
        )?))),
        "file-exists" => Ok(VerifierExpr::Predicate(VerifierPredicate::PathExists(
            SandboxPath::parse(string_argument(node, 0, "path")?)?,
        ))),
        "file-missing" => Ok(VerifierExpr::Predicate(VerifierPredicate::PathMissing(
            SandboxPath::parse(string_argument(node, 0, "path")?)?,
        ))),
        "file-read" => Ok(VerifierExpr::Predicate(VerifierPredicate::FileRead {
            path: SandboxPath::parse(string_argument(node, 0, "path")?)?,
            expected: string_argument(node, 1, "expected text")?,
        })),
        "file-contains" => Ok(VerifierExpr::Predicate(VerifierPredicate::FileContains {
            path: SandboxPath::parse(string_argument(node, 0, "path")?)?,
            needle: string_argument(node, 1, "expected text")?,
        })),
        "file-equals" => Ok(VerifierExpr::Predicate(VerifierPredicate::FileEquals {
            path: SandboxPath::parse(string_argument(node, 0, "path")?)?,
            expected: string_argument(node, 1, "expected text")?.into_bytes(),
        })),
        "cwd" => Ok(VerifierExpr::Predicate(
            VerifierPredicate::WorkingDirectory(SandboxPath::parse(string_argument(
                node, 0, "path",
            )?)?),
        )),
        other => Err(CompileError::UnknownNode(other.to_owned())),
    }
}

fn parse_hints(node: Option<&KdlNode>) -> Result<Vec<Hint>, CompileError> {
    let Some(node) = node else {
        return Ok(Vec::new());
    };
    let mut hints = Vec::new();
    for hint in child_nodes(Some(node)) {
        if hint.name().value() != "hint" {
            continue;
        }
        let level = hint
            .get("level")
            .and_then(|value| match value {
                KdlValue::Integer(value) => i64::try_from(*value).ok(),
                _ => None,
            })
            .ok_or(CompileError::MissingValue("hint level".to_owned()))?;
        hints.push(Hint {
            level: u8::try_from(level)
                .map_err(|_| CompileError::InvalidValue("hint level must fit in u8".to_owned()))?,
            text: child_required_string(child_map(hint).get("text"), "text")?,
            doc: child_map(hint)
                .get("doc")
                .and_then(|node| string_argument(node, 0, "doc ID").ok()),
        });
    }
    hints.sort_by_key(|hint| hint.level);
    Ok(hints)
}

fn parse_solution(node: Option<&KdlNode>) -> Result<CanonicalSolution, CompileError> {
    let Some(node) = node else {
        return Err(CompileError::MissingNode("solution"));
    };
    let children = child_nodes(Some(node));
    let concept = children
        .iter()
        .find(|child| child.name().value() == "concept")
        .map(|child| string_argument(child, 0, "concept"))
        .transpose()?
        .ok_or(CompileError::MissingNode("solution concept"))?;
    let commands = children
        .iter()
        .filter(|child| child.name().value() == "command")
        .map(|child| string_argument(child, 0, "command"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CanonicalSolution {
        concept,
        commands,
        why: children
            .iter()
            .filter(|child| child.name().value() == "why")
            .map(|child| string_argument(child, 0, "why"))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn validate_spec(spec: &ChallengeSpec) -> Result<(), CompileError> {
    if spec.meta.title.trim().is_empty() || spec.objective.text.trim().is_empty() {
        return Err(CompileError::InvalidValue(
            "title and objective must not be empty".to_owned(),
        ));
    }
    if spec
        .hints
        .windows(2)
        .any(|pair| pair[0].level == pair[1].level)
    {
        return Err(CompileError::InvalidValue(
            "hint levels must be unique".to_owned(),
        ));
    }
    if spec.solution.commands.is_empty() {
        return Err(CompileError::InvalidValue(
            "solution must contain at least one command".to_owned(),
        ));
    }
    validate_solution_tools(spec)?;
    Ok(())
}

fn validate_solution_tools(spec: &ChallengeSpec) -> Result<(), CompileError> {
    for command in &spec.solution.commands {
        let mut command_position = true;
        for token in command.split_whitespace() {
            if matches!(token, "|" | "||" | "&&" | ";") {
                command_position = true;
                continue;
            }
            if token.starts_with(['>', '<']) || token.ends_with(['>', '<']) {
                command_position = false;
                continue;
            }
            if !command_position || token.contains('=') {
                continue;
            }
            let tool = token
                .trim_matches(['\'', '"', '`'])
                .rsplit('/')
                .next()
                .unwrap_or(token);
            if !is_shell_builtin(tool)
                && !spec
                    .environment
                    .allowed_tools
                    .contains(&ToolId::new(tool).map_err(|_| {
                        CompileError::InvalidValue(format!(
                            "solution command uses invalid tool `{tool}`"
                        ))
                    })?)
            {
                return Err(CompileError::InvalidValue(format!(
                    "canonical solution for {} uses undeclared tool `{tool}`",
                    spec.id
                )));
            }
            command_position = false;
        }
    }
    Ok(())
}

fn is_shell_builtin(tool: &str) -> bool {
    matches!(
        tool,
        ":" | "."
            | "alias"
            | "bg"
            | "break"
            | "builtin"
            | "cd"
            | "command"
            | "continue"
            | "declare"
            | "dirs"
            | "echo"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "fc"
            | "fg"
            | "getopts"
            | "hash"
            | "help"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "printf"
            | "pwd"
            | "read"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "source"
            | "test"
            | "times"
            | "trap"
            | "type"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "wait"
    )
}

fn child_map(node: &KdlNode) -> BTreeMap<String, KdlNode> {
    child_nodes(Some(node))
        .iter()
        .map(|child| (child.name().value().to_owned(), (*child).clone()))
        .collect()
}

fn child_nodes(node: Option<&KdlNode>) -> Vec<&KdlNode> {
    node.and_then(KdlNode::children)
        .map(|document| document.nodes().iter().collect())
        .unwrap_or_default()
}

fn required_string(
    children: &BTreeMap<String, KdlNode>,
    name: &'static str,
) -> Result<String, CompileError> {
    children
        .get(name)
        .map(|node| string_argument(node, 0, name))
        .transpose()?
        .ok_or(CompileError::MissingNode(name))
}

fn child_required_string(
    node: Option<&KdlNode>,
    name: &'static str,
) -> Result<String, CompileError> {
    node.map(|node| string_argument(node, 0, name))
        .transpose()?
        .ok_or(CompileError::MissingNode(name))
}

fn optional_string(children: &BTreeMap<String, KdlNode>, name: &str) -> Option<String> {
    children
        .get(name)
        .and_then(|node| string_argument(node, 0, name).ok())
}

fn optional_integer(children: &BTreeMap<String, KdlNode>, name: &str) -> Option<i64> {
    children
        .get(name)
        .and_then(|node| integer_argument(node, 0, name).ok())
}

fn string_argument(
    node: &KdlNode,
    index: usize,
    description: &str,
) -> Result<String, CompileError> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .nth(index)
        .and_then(|entry| match entry.value() {
            KdlValue::String(value) => Some(Ok(value.clone())),
            _ => None,
        })
        .unwrap_or_else(|| Err(CompileError::MissingValue(description.to_owned())))
}

fn integer_argument(node: &KdlNode, index: usize, description: &str) -> Result<i64, CompileError> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .nth(index)
        .and_then(|entry| match entry.value() {
            KdlValue::Integer(value) => {
                Some(i64::try_from(*value).map_err(|_| {
                    CompileError::InvalidValue(format!("{description} must fit in i64"))
                }))
            }
            _ => None,
        })
        .unwrap_or_else(|| Err(CompileError::MissingValue(description.to_owned())))
}

/// Errors produced while compiling authored content.
#[derive(Debug, Error)]
pub enum CompileError {
    /// KDL parser error.
    #[error("KDL parse error: {0}")]
    Parse(String),
    /// Filesystem read error.
    #[error("content I/O error: {0}")]
    Io(String),
    /// An invalid typed value.
    #[error("invalid content value: {0}")]
    InvalidValue(String),
    /// An expected node is absent.
    #[error("missing required node {0}")]
    MissingNode(&'static str),
    /// An expected value is absent or has the wrong type.
    #[error("missing or invalid {0}")]
    MissingValue(String),
    /// An unknown node was encountered.
    #[error("unknown content node {0}")]
    UnknownNode(String),
    /// An identifier failed domain validation.
    #[error(transparent)]
    Identifier(#[from] learn_core::IdentifierError),
    /// A path failed sandbox validation.
    #[error(transparent)]
    Path(#[from] learn_core::PathError),
    /// A challenge ID appears more than once.
    #[error("duplicate challenge {0}")]
    DuplicateChallenge(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
challenge "fs.find.lost-note" version=1 {
    title "The Lost Note"
    summary "Find a file."
    difficulty "beginner"
    estimated-minutes 4
    skills {
        teaches "fs.search.find"
    }
    state "ephemeral"
    shell "bash"
    capabilities {
        network "none"
        tools {
            allow "find"
            allow "cat"
        }
    }
    objective {
        text "Find the note."
    }
    setup {
        mkdir "~/archive/notes"
        write "~/archive/notes/treasure.txt" {
            text "You found it."
        }
    }
    verify {
        all {
            file-read "~/archive/notes/treasure.txt" "You found it."
        }
    }
    hint level=1 {
        text "Search the tree."
    }
    solution {
        concept "Use find."
        command "find ~/archive -name treasure.txt"
    }
}
"#;

    #[test]
    fn compiles_typed_challenge() {
        let challenges = compile_source(SOURCE, "test.kdl").expect("compile");
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].spec.id.as_str(), "fs.find.lost-note");
        assert_eq!(challenges[0].spec.setup.len(), 2);
        assert_eq!(challenges[0].spec.environment.allowed_tools.len(), 2);
    }

    #[test]
    fn rejects_absolute_path_outside_home() {
        let source = SOURCE.replace("~/archive/notes", "/etc");
        assert!(matches!(
            compile_source(&source, "test.kdl"),
            Err(CompileError::Path(_))
        ));
    }

    #[test]
    fn rejects_canonical_commands_outside_declared_tools() {
        let source = SOURCE.replace(
            "find ~/archive -name treasure.txt",
            "unknown-tool ~/archive",
        );
        assert!(matches!(
            compile_source(&source, "test.kdl"),
            Err(CompileError::InvalidValue(message))
                if message.contains("undeclared tool `unknown-tool`")
        ));
    }
}
