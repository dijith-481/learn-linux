#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Pure domain types and deterministic learning logic.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{Display, Formatter};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A validated identifier used by a challenge, skill, or track.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Identifier(String);

impl Identifier {
    /// Create an identifier after validating its stable wire representation.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 128
            && value.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '.' | '-' | '_')
            })
            && !value.starts_with(['.', '-', '_'])
            && !value.ends_with(['.', '-', '_']);
        if valid {
            Ok(Self(value))
        } else {
            Err(IdentifierError(value))
        }
    }

    /// Return the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for Identifier {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Error returned for malformed identifiers.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(
    "invalid identifier `{0}`; use lowercase letters, digits, '.', '-' or '_' without a leading/trailing separator"
)]
pub struct IdentifierError(String);

macro_rules! typed_identifier {
    ($name:ident) => {
        #[doc = concat!("A validated ", stringify!($name), " identifier.")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        pub struct $name(Identifier);

        impl $name {
            /// Parse a stable identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                Identifier::new(value).map(Self)
            }

            /// Borrow the stable identifier.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

typed_identifier!(ChallengeId);
typed_identifier!(SkillId);
typed_identifier!(TrackId);
typed_identifier!(DocId);
typed_identifier!(HintId);
typed_identifier!(ToolId);
typed_identifier!(SessionId);

/// A path in the learner's sandbox, never a host filesystem path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SandboxPath(String);

impl SandboxPath {
    /// Parse an absolute sandbox path or a path relative to the learner home.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PathError> {
        let raw = value.as_ref();
        if raw.is_empty() || raw.contains('\0') {
            return Err(PathError::Invalid(raw.to_owned()));
        }
        let normalized = if let Some(home_relative) = raw.strip_prefix("~/") {
            format!("/home/learner/{home_relative}")
        } else if raw == "~" {
            "/home/learner".to_owned()
        } else if raw.starts_with('/') {
            raw.to_owned()
        } else {
            format!("/home/learner/{raw}")
        };
        let mut components = Vec::new();
        for component in Path::new(&normalized).components() {
            let component = component.as_os_str().to_string_lossy();
            match component.as_ref() {
                "/" | "" | "." => {}
                ".." => {
                    if components.pop().is_none() {
                        return Err(PathError::Escape(raw.to_owned()));
                    }
                }
                part => components.push(part.to_owned()),
            }
        }
        let normalized = format!("/{}", components.join("/"));
        if normalized != "/home/learner" && !normalized.starts_with("/home/learner/") {
            return Err(PathError::OutsideHome(normalized));
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized absolute path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for SandboxPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned for invalid challenge paths.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PathError {
    /// Empty or NUL-containing input.
    #[error("invalid sandbox path `{0}`")]
    Invalid(String),
    /// A normalized path attempted to leave its permitted namespace.
    #[error("sandbox path `{0}` escapes its namespace")]
    Escape(String),
    /// A path outside the learner home is not permitted by this operation.
    #[error("sandbox path `{0}` is outside the learner home")]
    OutsideHome(String),
}

/// Whether a challenge's filesystem is reset between sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StateMode {
    /// Recreate all mutable challenge state for each session.
    Ephemeral,
    /// Preserve the learner home while resetting challenge fixtures.
    PersistentHome,
    /// Explicit long-lived advanced environment.
    PersistentWorld,
}

/// Network access available to a challenge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NetworkPolicy {
    /// No network interfaces other than the kernel's isolated loopback.
    None,
    /// Isolated loopback only.
    Loopback,
    /// The host network. Only for explicitly trusted lessons.
    HostNetwork,
}

/// Shell used by a challenge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ShellKind {
    /// The canonical real Bash runtime.
    Bash,
}

/// How strongly the runtime limits external executables.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ToolRestrictionStrength {
    /// No executable hiding; the UI guides the learner.
    Guidance,
    /// Only declared executables are visible in the challenge namespace.
    ExternalExecutableVisibility,
}

/// Challenge runtime requirements.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentSpec {
    /// Reset policy.
    pub state: StateMode,
    /// Shell implementation.
    pub shell: ShellKind,
    /// Network policy.
    pub network: NetworkPolicy,
    /// Allowed command names.
    pub allowed_tools: BTreeSet<ToolId>,
    /// Restriction behavior.
    pub restriction: ToolRestrictionStrength,
}

impl Default for EnvironmentSpec {
    fn default() -> Self {
        Self {
            state: StateMode::Ephemeral,
            shell: ShellKind::Bash,
            network: NetworkPolicy::None,
            allowed_tools: BTreeSet::new(),
            restriction: ToolRestrictionStrength::Guidance,
        }
    }
}

/// A typed setup operation applied to a fresh challenge home.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SetupAction {
    /// Create a directory.
    CreateDir {
        /// Destination directory.
        path: SandboxPath,
    },
    /// Write deterministic bytes to a file.
    WriteFile {
        /// Destination file.
        path: SandboxPath,
        /// File contents.
        contents: Vec<u8>,
    },
    /// Set a file mode represented as Unix permission bits.
    SetMode {
        /// Target path.
        path: SandboxPath,
        /// Unix permission bits.
        mode: u32,
    },
}

/// A verifier predicate based on resulting sandbox state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VerifierPredicate {
    /// A path exists.
    PathExists(SandboxPath),
    /// A path does not exist.
    PathMissing(SandboxPath),
    /// A regular file contains bytes/text.
    FileContains {
        /// File path.
        path: SandboxPath,
        /// Required text.
        needle: String,
    },
    /// The learner has read a regular file and it contains bytes/text.
    FileRead {
        /// File path.
        path: SandboxPath,
        /// Required text.
        expected: String,
    },
    /// A file exactly equals bytes.
    FileEquals {
        /// File path.
        path: SandboxPath,
        /// Exact expected contents.
        expected: Vec<u8>,
    },
    /// The shell is currently in a directory.
    WorkingDirectory(SandboxPath),
}

/// Boolean composition of verifier predicates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VerifierExpr {
    /// Every child must pass.
    All(Vec<VerifierExpr>),
    /// At least one child must pass.
    Any(Vec<VerifierExpr>),
    /// Invert a child.
    Not(Box<VerifierExpr>),
    /// Evaluate one predicate.
    Predicate(VerifierPredicate),
}

/// Runtime-observed state used by the pure verifier evaluator.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObservedState {
    /// Existing regular paths.
    pub existing_paths: BTreeSet<SandboxPath>,
    /// File contents keyed by sandbox path.
    pub files: BTreeMap<SandboxPath, Vec<u8>>,
    /// Files observed being read by the learner's shell session.
    pub read_paths: BTreeSet<SandboxPath>,
    /// Last reported working directory.
    pub working_directory: Option<SandboxPath>,
}

impl VerifierExpr {
    /// Evaluate this expression against observed state.
    pub fn evaluate(&self, state: &ObservedState) -> bool {
        match self {
            Self::All(children) => children.iter().all(|child| child.evaluate(state)),
            Self::Any(children) => children.iter().any(|child| child.evaluate(state)),
            Self::Not(child) => !child.evaluate(state),
            Self::Predicate(predicate) => match predicate {
                VerifierPredicate::PathExists(path) => state.existing_paths.contains(path),
                VerifierPredicate::PathMissing(path) => !state.existing_paths.contains(path),
                VerifierPredicate::FileContains { path, needle } => state
                    .files
                    .get(path)
                    .is_some_and(|contents| String::from_utf8_lossy(contents).contains(needle)),
                VerifierPredicate::FileRead { path, expected } => {
                    state.read_paths.contains(path)
                        && state.files.get(path).is_some_and(|contents| {
                            String::from_utf8_lossy(contents).contains(expected)
                        })
                }
                VerifierPredicate::FileEquals { path, expected } => state
                    .files
                    .get(path)
                    .is_some_and(|contents| contents == expected),
                VerifierPredicate::WorkingDirectory(path) => {
                    state.working_directory.as_ref() == Some(path)
                }
            },
        }
    }
}

/// A skill in the directed learning graph.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// Stable skill ID.
    pub id: SkillId,
    /// Human-readable title.
    pub title: String,
    /// Short explanation.
    pub description: String,
    /// Required skills.
    pub prerequisites: Vec<SkillId>,
    /// Curriculum track.
    pub track: TrackId,
}

/// An immutable skill graph.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillGraph {
    skills: BTreeMap<SkillId, Skill>,
}

impl SkillGraph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill, rejecting duplicate IDs.
    pub fn insert(&mut self, skill: Skill) -> Result<(), GraphError> {
        if self.skills.contains_key(&skill.id) {
            return Err(GraphError::Duplicate(skill.id.to_string()));
        }
        self.skills.insert(skill.id.clone(), skill);
        Ok(())
    }

    /// Validate references and acyclicity.
    pub fn validate(&self) -> Result<(), GraphError> {
        for skill in self.skills.values() {
            for prerequisite in &skill.prerequisites {
                if !self.skills.contains_key(prerequisite) {
                    return Err(GraphError::MissingReference {
                        skill: skill.id.to_string(),
                        prerequisite: prerequisite.to_string(),
                    });
                }
            }
        }
        for skill in self.skills.keys() {
            let mut visiting = BTreeSet::new();
            self.visit(skill, &mut visiting)?;
        }
        Ok(())
    }

    fn visit(&self, id: &SkillId, visiting: &mut BTreeSet<SkillId>) -> Result<(), GraphError> {
        if !visiting.insert(id.clone()) {
            return Err(GraphError::Cycle(id.to_string()));
        }
        if let Some(skill) = self.skills.get(id) {
            for prerequisite in &skill.prerequisites {
                self.visit(prerequisite, visiting)?;
            }
        }
        visiting.remove(id);
        Ok(())
    }

    /// Return skills whose prerequisites are all demonstrated and which are not completed.
    pub fn eligible(
        &self,
        demonstrated: &BTreeSet<SkillId>,
        completed: &BTreeSet<SkillId>,
    ) -> Vec<SkillId> {
        self.skills
            .values()
            .filter(|skill| {
                !completed.contains(&skill.id)
                    && skill
                        .prerequisites
                        .iter()
                        .all(|id| demonstrated.contains(id))
            })
            .map(|skill| skill.id.clone())
            .collect()
    }

    /// Return all skills in stable order.
    pub fn skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }
}

/// Graph validation error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GraphError {
    /// An ID was inserted twice.
    #[error("duplicate skill `{0}`")]
    Duplicate(String),
    /// A prerequisite is absent.
    #[error("skill `{skill}` references missing prerequisite `{prerequisite}`")]
    MissingReference {
        /// Skill containing the invalid reference.
        skill: String,
        /// Missing prerequisite.
        prerequisite: String,
    },
    /// A prerequisite cycle exists.
    #[error("skill prerequisite cycle includes `{0}`")]
    Cycle(String),
}

/// Completion state persisted for each challenge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CompletionStatus {
    /// Never opened.
    Locked,
    /// Started but not verified.
    InProgress,
    /// Verified successfully.
    Completed,
}

/// Persistent challenge progress.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChallengeProgress {
    /// Challenge identity.
    pub challenge_id: ChallengeId,
    /// Current completion state.
    pub status: CompletionStatus,
    /// Number of attempts.
    pub attempts: u32,
    /// Hint IDs consumed.
    pub hints_used: Vec<HintId>,
}

/// Find a deterministic topological order of skills.
pub fn topological_order(graph: &SkillGraph) -> Result<Vec<SkillId>, GraphError> {
    graph.validate()?;
    let mut indegree = BTreeMap::<SkillId, usize>::new();
    let mut edges = BTreeMap::<SkillId, Vec<SkillId>>::new();
    for skill in graph.skills() {
        indegree.insert(skill.id.clone(), skill.prerequisites.len());
        for prerequisite in &skill.prerequisites {
            edges
                .entry(prerequisite.clone())
                .or_default()
                .push(skill.id.clone());
        }
    }
    let mut queue = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect::<VecDeque<_>>();
    let mut order = Vec::with_capacity(indegree.len());
    while let Some(id) = queue.pop_front() {
        order.push(id.clone());
        if let Some(children) = edges.get(&id) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_home_relative_paths() {
        let path = SandboxPath::parse("~/notes/../answer.txt").expect("valid path");
        assert_eq!(path.as_str(), "/home/learner/answer.txt");
    }

    #[test]
    fn rejects_paths_outside_home() {
        assert!(matches!(
            SandboxPath::parse("/etc/passwd"),
            Err(PathError::OutsideHome(_))
        ));
        assert!(matches!(
            SandboxPath::parse("../../secret"),
            Err(PathError::Escape(_) | PathError::OutsideHome(_))
        ));
    }

    #[test]
    fn verifier_is_state_based() {
        let path = SandboxPath::parse("~/answer").expect("valid path");
        let expression = VerifierExpr::Predicate(VerifierPredicate::FileContains {
            path: path.clone(),
            needle: "done".to_owned(),
        });
        let state = ObservedState {
            existing_paths: [path.clone()].into_iter().collect(),
            files: [(path, b"done!".to_vec())].into_iter().collect(),
            read_paths: BTreeSet::new(),
            working_directory: None,
        };
        assert!(expression.evaluate(&state));
    }

    #[test]
    fn graph_rejects_cycles() {
        let track = TrackId::new("foundation").expect("valid ID");
        let first = SkillId::new("first").expect("valid ID");
        let second = SkillId::new("second").expect("valid ID");
        let mut graph = SkillGraph::new();
        graph
            .insert(Skill {
                id: first.clone(),
                title: "First".to_owned(),
                description: String::new(),
                prerequisites: vec![second.clone()],
                track: track.clone(),
            })
            .expect("insert");
        graph
            .insert(Skill {
                id: second,
                title: "Second".to_owned(),
                description: String::new(),
                prerequisites: vec![first],
                track,
            })
            .expect("insert");
        assert!(matches!(graph.validate(), Err(GraphError::Cycle(_))));
    }
}
