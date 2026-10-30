//! Explicit, invocation-local admission callbacks for the ordinary resolver.
use super::*;
use std::fmt;

/// The own scalar or immediate container shape of a borrowed summary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SummaryValue<'a> {
    /// Explicit null.
    Null,
    /// Exact boolean.
    Bool(bool),
    /// Exact decimal lexeme, without host numeric conversion.
    Number(&'a str),
    /// Borrowed text.
    String(&'a str),
    /// Sequence length, without descendants.
    Sequence(usize),
    /// Mapping length, without descendants.
    Mapping(usize),
}

#[derive(Clone, Copy)]
enum SummarySource<'a> {
    Resolved(&'a ResolvedNode<'a>),
    Default(&'a JsonValue, Option<&'a str>, Option<&'a str>),
}

/// A shallow read-only view, also usable before a schema default is expanded.
#[derive(Clone, Copy)]
pub struct NodeSummary<'a> {
    source: SummarySource<'a>,
}

impl<'a> NodeSummary<'a> {
    /// Returns the own value or container length.
    pub fn value(&self) -> SummaryValue<'a> {
        match self.source {
            SummarySource::Resolved(node) => resolved_summary(node),
            SummarySource::Default(value, _, _) => default_summary(value),
        }
    }
    /// Returns only this node's asserted tag.
    pub fn tag(&self) -> Option<&'a str> {
        match self.source {
            SummarySource::Resolved(node) => node.tag(),
            SummarySource::Default(_, tag, _) => tag,
        }
    }
    /// Returns only this node's explicit binding identifier.
    pub fn tag_binding(&self) -> Option<&'a str> {
        match self.source {
            SummarySource::Resolved(node) => node.tag_binding(),
            SummarySource::Default(_, _, binding) => binding,
        }
    }
    /// Iterates direct map fields; field containers expose only their length.
    pub fn fields(&self) -> SummaryFields<'a> {
        let source = match self.source {
            SummarySource::Resolved(node) => match &node.value {
                ResolvedValue::Mapping(fields) => FieldSource::Resolved(fields.iter()),
                _ => FieldSource::Empty,
            },
            SummarySource::Default(value, _, _) => match value {
                JsonValue::Object(fields) => FieldSource::Default(fields.iter()),
                _ => FieldSource::Empty,
            },
        };
        SummaryFields { source }
    }
}

enum FieldSource<'a> {
    Resolved(std::collections::btree_map::Iter<'a, Cow<'a, str>, ResolvedNode<'a>>),
    Default(serde_json::map::Iter<'a>),
    Empty,
}

/// Borrowed shallow map-field iterator; never exports descendant trees.
pub struct SummaryFields<'a> {
    source: FieldSource<'a>,
}
impl<'a> Iterator for SummaryFields<'a> {
    type Item = (&'a str, SummaryValue<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.source {
            FieldSource::Resolved(iter) => iter
                .next()
                .map(|(key, node)| (key.as_ref(), resolved_summary(node))),
            FieldSource::Default(iter) => iter
                .next()
                .map(|(key, value)| (key.as_str(), default_summary(value))),
            FieldSource::Empty => None,
        }
    }
}
fn resolved_summary<'a>(node: &'a ResolvedNode<'_>) -> SummaryValue<'a> {
    match &node.value {
        ResolvedValue::Null => SummaryValue::Null,
        ResolvedValue::Bool(value) => SummaryValue::Bool(*value),
        ResolvedValue::Number(value) => SummaryValue::Number(value.as_str()),
        ResolvedValue::String(value) => SummaryValue::String(value),
        ResolvedValue::Sequence(items) => SummaryValue::Sequence(items.len()),
        ResolvedValue::Mapping(fields) => SummaryValue::Mapping(fields.len()),
    }
}
fn default_summary(value: &JsonValue) -> SummaryValue<'_> {
    match value {
        JsonValue::Null => SummaryValue::Null,
        JsonValue::Bool(value) => SummaryValue::Bool(*value),
        JsonValue::Number(value) => SummaryValue::Number(value.as_str()),
        JsonValue::String(value) => SummaryValue::String(value),
        JsonValue::Array(items) => SummaryValue::Sequence(items.len()),
        JsonValue::Object(fields) => SummaryValue::Mapping(fields.len()),
    }
}

/// One bounded transformation or additional named contract.
#[derive(Debug, Clone)]
pub enum StageEdit {
    /// Replace a root or a leaf under an existing container.
    Replace {
        /// Destination RFC 6901 instance pointer.
        path: String,
        /// Exact semantic input admitted before applying the batch.
        value: SemanticNode,
    },
    /// Import a completed snapshot subtree, retaining its real provenance.
    Copy {
        /// Destination RFC 6901 instance pointer.
        path: String,
        /// Source pointer in the current completed snapshot.
        from: String,
    },
    /// Conjoin a named native contract at a data path.
    Expect {
        /// Instance pointer at which the contract must hold.
        path: String,
        /// Explicit entry-visible native type name.
        name: String,
    },
}
/// Applied copy-operation provenance for the current candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyEdge {
    /// Destination instance pointer.
    pub path: String,
    /// Source instance pointer in that operation's snapshot.
    pub from: String,
}
/// A synchronous materialization decision.
#[derive(Debug)]
pub enum StageOutcome<E> {
    /// Host transformations are complete; strict native validation still follows.
    Ready {
        /// Optional bounded host meaning that refines final candidate equality.
        meaning: Option<Vec<u8>>,
    },
    /// Atomically apply this nonempty, progressing batch.
    Edits(Vec<StageEdit>),
    /// Reject this candidate, without making callback execution fatal.
    Reject(E),
}
/// Distinguishes native failure, fatal callback failure, and explicit rejection.
#[derive(Debug)]
pub enum StageError<E> {
    /// Native admission, validation, resource, or edit-contract error.
    Resolve(ResolveError),
    /// Original fatal callback error.
    Callback(E),
    /// Original explicit candidate rejection.
    Rejected(E),
}
impl<E: fmt::Display> fmt::Display for StageError<E> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(error) => error.fmt(out),
            Self::Callback(error) => write!(out, "admission callback failed: {error}"),
            Self::Rejected(error) => write!(out, "admission candidate rejected: {error}"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for StageError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error),
            Self::Callback(error) | Self::Rejected(error) => Some(error),
        }
    }
}
/// An explicitly selected visitor; no registry or retained mutable draft exists.
pub trait AdmissionStage {
    /// Host-owned error identity.
    type Error;
    /// Classifies a shallow node before any ancestor default merge.
    fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, Self::Error>;
    /// Receives one completed, not-yet-checked candidate snapshot per round.
    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error>;
    /// Pure, path-independent refinement of native numeric equality.
    fn number_class(&mut self, _lexeme: &str) -> Result<u32, Self::Error> {
        Ok(0)
    }
}
/// Strictly checked winning data and the winning optional host meaning key.
#[derive(Debug)]
pub struct StagedResolution<'a> {
    pub(super) root: ResolvedNode<'a>,
    pub(super) meaning: Option<Vec<u8>>,
}
impl<'a> StagedResolution<'a> {
    /// Returns checked immutable data.
    pub fn root(&self) -> &ResolvedNode<'a> {
        &self.root
    }
    /// Returns the selected meaning, not a key emitted by an invalid candidate.
    pub fn meaning(&self) -> Option<&[u8]> {
        self.meaning.as_deref()
    }
}

// Erase only the callback error type inside the existing resolver. The adapter
// retains the original error, never translating it into a candidate violation.
pub(super) trait StageHooks {
    fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, ()>;
    fn number_class(&mut self, lexeme: &str) -> Result<u32, ()>;
    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<()>, ()>;
    fn failed(&self) -> bool;
}
struct StageAdapter<'s, S: AdmissionStage> {
    stage: &'s mut S,
    fatal: Option<S::Error>,
    rejected: Option<S::Error>,
}
impl<S: AdmissionStage> StageHooks for StageAdapter<'_, S> {
    fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> Result<bool, ()> {
        self.stage.defer(node, path).map_err(|error| {
            self.fatal = Some(error);
        })
    }
    fn number_class(&mut self, lexeme: &str) -> Result<u32, ()> {
        self.stage.number_class(lexeme).map_err(|error| {
            self.fatal = Some(error);
        })
    }
    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<()>, ()> {
        match self.stage.materialize(root, copies, round) {
            Ok(StageOutcome::Ready { meaning }) => Ok(StageOutcome::Ready { meaning }),
            Ok(StageOutcome::Edits(edits)) => Ok(StageOutcome::Edits(edits)),
            Ok(StageOutcome::Reject(error)) => {
                if self.rejected.is_none() {
                    self.rejected = Some(error);
                }
                Ok(StageOutcome::Reject(()))
            }
            Err(error) => {
                self.fatal = Some(error);
                Err(())
            }
        }
    }
    fn failed(&self) -> bool {
        self.fatal.is_some()
    }
}

impl Schemas {
    /// Resolves source through an explicit bounded admission stage.
    ///
    /// # Errors
    ///
    /// Returns native admission/validation failures, original fatal callback
    /// errors, or an original rejection when every candidate is rejected.
    pub fn resolve_staged<'a, S: AdmissionStage>(
        &'a self,
        document: &'a Document,
        expected: Option<&str>,
        initial_edits: &[StageEdit],
        stage: &mut S,
    ) -> Result<StagedResolution<'a>, StageError<S::Error>> {
        self.resolve_staged_with_limits(
            document,
            expected,
            initial_edits,
            ResolutionLimits::default(),
            stage,
        )
    }
    /// Resolves source with cumulative bounds shared by every replay and round.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::resolve_staged`], including cumulative
    /// resource exhaustion across callback exports, imports, and replay.
    pub fn resolve_staged_with_limits<'a, S: AdmissionStage>(
        &'a self,
        document: &'a Document,
        expected: Option<&str>,
        initial_edits: &[StageEdit],
        limits: ResolutionLimits,
        stage: &mut S,
    ) -> Result<StagedResolution<'a>, StageError<S::Error>> {
        self.staged(
            CandidateSource::Document(document),
            expected,
            initial_edits,
            limits,
            stage,
        )
    }
    /// Resolves exact semantic input through the same admission stage.
    ///
    /// # Errors
    ///
    /// Returns representation, binding, schema, ambiguity, resource, or original
    /// callback failures without inventing source positions.
    pub fn resolve_semantic_staged<'a, S: AdmissionStage>(
        &'a self,
        node: &'a SemanticNode,
        expected: Option<&str>,
        initial_edits: &[StageEdit],
        stage: &mut S,
    ) -> Result<StagedResolution<'a>, StageError<S::Error>> {
        self.resolve_semantic_staged_with_limits(
            node,
            expected,
            initial_edits,
            ResolutionLimits::default(),
            stage,
        )
    }
    /// Resolves semantic input with cumulative invocation-wide bounds.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::resolve_semantic_staged`], bounded by
    /// the supplied invocation-wide limits.
    pub fn resolve_semantic_staged_with_limits<'a, S: AdmissionStage>(
        &'a self,
        node: &'a SemanticNode,
        expected: Option<&str>,
        initial_edits: &[StageEdit],
        limits: ResolutionLimits,
        stage: &mut S,
    ) -> Result<StagedResolution<'a>, StageError<S::Error>> {
        self.staged(
            CandidateSource::Semantic(node),
            expected,
            initial_edits,
            limits,
            stage,
        )
    }
    fn staged<'a, S: AdmissionStage>(
        &'a self,
        source: CandidateSource<'a>,
        expected: Option<&str>,
        initial_edits: &[StageEdit],
        limits: ResolutionLimits,
        stage: &mut S,
    ) -> Result<StagedResolution<'a>, StageError<S::Error>> {
        let mut adapter = StageAdapter {
            stage,
            fatal: None,
            rejected: None,
        };
        let result = Resolver {
            schemas: self,
            budget: Budget {
                limits,
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: Some(&mut adapter),
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        }
        .run_staged(source, expected, initial_edits);
        if let Some(error) = adapter.fatal {
            return Err(StageError::Callback(error));
        }
        match result {
            Ok(Some(result)) => Ok(result),
            Ok(None) => Err(StageError::Rejected(
                adapter
                    .rejected
                    .expect("rejected candidate retains its error"),
            )),
            Err(error) => Err(StageError::Resolve(error)),
        }
    }
}

impl Budget<'_> {
    fn callback_failure(node: &ResolvedNode<'_>, path: &str) -> ResolveError {
        failure(
            ResolveErrorKind::InvalidStage,
            node,
            "admission callback failed",
            None,
            path,
        )
    }
    pub(super) fn classify(
        &mut self,
        node: &mut ResolvedNode<'_>,
        path: &str,
    ) -> Result<(), ResolveError> {
        if self.stage.is_none() {
            return Ok(());
        }
        self.step(node, path)?;
        self.charge_summary(
            NodeSummary {
                source: SummarySource::Resolved(node),
            },
            node,
            path,
        )?;
        node.opaque = self
            .stage
            .as_mut()
            .expect("selected stage")
            .defer(
                NodeSummary {
                    source: SummarySource::Resolved(node),
                },
                path,
            )
            .map_err(|()| Self::callback_failure(node, path))?;
        self.classify_number(node, path)
    }
    pub(super) fn classify_number(
        &mut self,
        node: &mut ResolvedNode<'_>,
        path: &str,
    ) -> Result<(), ResolveError> {
        if let ResolvedValue::Number(number) = &node.value
            && let Some(stage) = self.stage.as_mut()
        {
            node.number_class = stage
                .number_class(number.as_str())
                .map_err(|()| Self::callback_failure(node, path))?;
        }
        Ok(())
    }
    pub(super) fn defer_default(
        &mut self,
        value: &JsonValue,
        tag: Option<&ResolvedTag<'_>>,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<bool, ResolveError> {
        if self.stage.is_none() {
            return Ok(false);
        }
        self.step(node, path)?;
        let summary = NodeSummary {
            source: SummarySource::Default(
                value,
                tag.map(|tag| tag.name.as_ref()),
                tag.and_then(|tag| tag.binding.as_deref()),
            ),
        };
        self.charge_summary(summary, node, path)?;
        self.stage
            .as_mut()
            .expect("selected stage")
            .defer(summary, path)
            .map_err(|()| Self::callback_failure(node, path))
    }
    fn charge_summary(
        &mut self,
        summary: NodeSummary<'_>,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<(), ResolveError> {
        self.add_node(node, path)?;
        self.add_bytes(node, path, path)?;
        if let Some(tag) = summary.tag() {
            self.add_bytes(node, tag, path)?;
        }
        if let Some(binding) = summary.tag_binding() {
            self.add_bytes(node, binding, path)?;
        }
        self.charge_summary_value(summary.value(), node, path)?;
        for (key, value) in summary.fields() {
            self.add_node(node, path)?;
            self.add_bytes(node, key, path)?;
            self.charge_summary_value(value, node, path)?;
        }
        Ok(())
    }
    fn charge_summary_value(
        &mut self,
        value: SummaryValue<'_>,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<(), ResolveError> {
        match value {
            SummaryValue::String(text) | SummaryValue::Number(text) => {
                self.add_bytes(node, text, path)
            }
            _ => Ok(()),
        }
    }
}

// Fresh edits and newly unblocked subtrees share completion eligibility.
// Completed ancestors only project presets toward them; the closest active
// root or completed-copy fence controls each descendant independently.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DefaultScope {
    Skip,
    Project,
    Complete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObligationFreshness {
    Existing,
    Pending,
    New,
}

#[derive(Default)]
pub(super) struct PendingDefaults {
    active: Vec<String>,
    copied: Vec<String>,
}

impl PendingDefaults {
    pub(super) fn scope(
        &self,
        path: &str,
        node: &ResolvedNode<'_>,
        budget: &mut Budget<'_>,
    ) -> Result<DefaultScope, ResolveError> {
        budget.charge_work(
            self.active.len().saturating_add(self.copied.len()),
            node,
            path,
        )?;
        let mut active = None;
        let mut projects = false;
        for root in &self.active {
            if contains_path(root, path) {
                active = active.max(Some(root.len()));
            } else if contains_path(path, root) {
                projects = true;
            }
        }
        let copied = self
            .copied
            .iter()
            .filter(|root| contains_path(root, path))
            .map(String::len)
            .max();
        Ok(if active.is_some() && active >= copied {
            DefaultScope::Complete
        } else if projects {
            DefaultScope::Project
        } else {
            DefaultScope::Skip
        })
    }

    fn add_scope(
        scopes: &mut Vec<String>,
        path: &str,
        node: &ResolvedNode<'_>,
        budget: &mut Budget<'_>,
    ) -> Result<(), ResolveError> {
        budget.charge_work(scopes.len(), node, path)?;
        if scopes.iter().any(|root| root == path) {
            return Ok(());
        }
        budget.add_node(node, path)?;
        budget.add_bytes(node, path, path)?;
        // An ancestor cannot subsume a nested exception across a copy fence.
        scopes.push(path.to_owned());
        Ok(())
    }
}

impl<'a> Resolver<'a, '_> {
    fn run_staged(
        &mut self,
        source: CandidateSource<'a>,
        expected: Option<&str>,
        initial: &[StageEdit],
    ) -> Result<Option<StagedResolution<'a>>, ResolveError> {
        let root = match &source {
            CandidateSource::Document(document) => {
                from_input(document, document.root(), 0, "", &mut self.budget)?
            }
            CandidateSource::Semantic(node) => from_semantic(node, 0, "", &mut self.budget)?,
            CandidateSource::Resolved(_) => {
                unreachable!("staged entries admit source or semantic data")
            }
        };
        self.known_tags(&root, "")?;
        let views = self.root_views(&root, expected)?;
        self.select_candidates(
            &views,
            Some(root),
            source,
            CandidateRequirement::Unique,
            initial,
        )
    }

    pub(super) fn stage_candidate(
        &mut self,
        mut root: ResolvedNode<'a>,
        views: &[SchemaView<'a>],
    ) -> Result<Option<StagedResolution<'a>>, ResolveError> {
        let mut copies = Vec::new();
        let mut round = 0usize;
        loop {
            root = self.complete(root, views, "", CompletionLevel::existing(0))?;
            // Pin generated assertions for semantic round trips, without
            // certifying the still-unchecked candidate's native constraints.
            self.admit_tags(&mut root, "", 0, TagAdmission::PinCandidate)?;
            // Bound the host's full snapshot and complete applied-copy export.
            self.charge_candidate_copy(&root, "", 0)?;
            for edge in &copies {
                let edge: &CopyEdge = edge;
                self.budget.step(&root, &edge.path)?;
                self.budget.add_bytes(&root, &edge.path, "")?;
                self.budget.add_bytes(&root, &edge.from, "")?;
            }
            let outcome = self
                .budget
                .stage
                .as_mut()
                .expect("selected stage")
                .materialize(&root, &copies, round)
                .map_err(|()| Budget::callback_failure(&root, ""))?;
            match outcome {
                StageOutcome::Reject(()) => return Ok(None),
                StageOutcome::Ready { meaning } => {
                    if let Some(key) = &meaning {
                        self.budget.add_byte_count(&root, key.len(), "")?;
                    }
                    let root = self.finish_candidate(views, root)?;
                    for (path, contracts) in &self.contracts {
                        let node = root.at_pointer(path).ok_or_else(|| {
                            failure(
                                ResolveErrorKind::SchemaViolation,
                                &root,
                                "an obligated value is missing",
                                None,
                                path,
                            )
                        })?;
                        for view in contracts {
                            Self::check_node(node, view, path, &mut self.budget)?;
                        }
                    }
                    return Ok(Some(StagedResolution { root, meaning }));
                }
                StageOutcome::Edits(edits) => {
                    let edges = self.apply_edits(&mut root, &edits, false)?;
                    // apply_edits preserves completed Copy fences and resumes
                    // only active edits or newly available subtrees.
                    copies.extend(edges);
                    round = round.checked_add(1).ok_or_else(|| {
                        failure(
                            ResolveErrorKind::LimitExceeded,
                            &root,
                            "materialization round limit exceeded",
                            None,
                            "",
                        )
                    })?;
                }
            }
        }
    }

    pub(super) fn apply_edits(
        &mut self,
        root: &mut ResolvedNode<'a>,
        edits: &[StageEdit],
        initial: bool,
    ) -> Result<Vec<CopyEdge>, ResolveError> {
        if edits.is_empty() {
            return if initial {
                Ok(Vec::new())
            } else {
                Err(invalid(root, "", "empty edit batch"))
            };
        }
        self.budget.charge_work(edits.len(), root, "")?;
        self.budget.semantic_children(root, edits.len(), "")?;
        let mut destinations: Vec<&str> = Vec::new();
        for edit in edits {
            let path = edit_path(edit);
            self.budget.add_bytes(root, path, path)?;
            validate_pointer(root, path)?;
            match edit {
                StageEdit::Expect { name, .. } => {
                    self.budget.add_bytes(root, name, path)?;
                }
                StageEdit::Copy { from, .. } => {
                    if initial {
                        return Err(invalid(
                            root,
                            path,
                            "initial edits cannot copy incomplete values",
                        ));
                    }
                    self.budget.add_bytes(root, from, path)?;
                    validate_pointer(root, from)?;
                    validate_destination(root, path)?;
                    destinations.push(path);
                }
                StageEdit::Replace { .. } => {
                    validate_destination(root, path)?;
                    destinations.push(path);
                }
            }
        }
        for (index, path) in destinations.iter().enumerate() {
            for other in &destinations[..index] {
                self.budget.step(root, path)?;
                if overlaps(path, other) {
                    return Err(invalid(root, path, "overlapping data destinations"));
                }
            }
        }
        let mut replacements = Vec::new();
        let mut active = Vec::new();
        let mut obligations: Vec<(String, SchemaView<'a>)> = Vec::new();
        let mut edges = Vec::new();
        let mut progress = false;
        for edit in edits {
            let path = edit_path(edit);
            if let Some((_, part)) = path.rsplit_once('/') {
                let key = crate::pointer::decode_segment(part).expect("admitted escape");
                self.budget.add_bytes(root, &key, path)?;
            }
            self.budget.add_node(root, path)?;
            if let StageEdit::Expect { name, .. } = edit {
                let view = self.schemas.type_schema(name).ok_or_else(|| {
                    failure(
                        ResolveErrorKind::UnknownExpectedType,
                        root.at_pointer(path).unwrap_or(root),
                        format!("no schema is bound to expected type {name:?}"),
                        None,
                        path,
                    )
                })?;
                let freshness = self.add_obligation(&mut obligations, path, view, root)?;
                progress |= freshness == ObligationFreshness::New;
                // Same-batch Copy assertions must not consume a new Expect's
                // activation; only the pre-batch obligations determine freshness.
                if !initial && freshness != ObligationFreshness::Existing {
                    PendingDefaults::add_scope(&mut active, path, root, &mut self.budget)?;
                }
                continue;
            }
            // Conserving the snapshot's existing assertions is not itself
            // progress. Seed them before importing any new conjuncts.
            if let Some(old) = root.at_pointer(path) {
                self.collect_assertions(old, path, true, &mut obligations)?;
            }
            let prior_obligations = obligations.len();
            let mut value = match edit {
                StageEdit::Replace { value, .. } => {
                    let node = from_semantic(value, pointer_depth(path), path, &mut self.budget)?;
                    self.known_tags(&node, path)?;
                    own_semantic_node(node)
                }
                StageEdit::Copy { from, .. } => {
                    if destinations
                        .iter()
                        .any(|destination| overlaps(destination, from))
                    {
                        return Err(invalid(
                            root,
                            path,
                            "copy source is modified in the same batch",
                        ));
                    }
                    let source = self.completed_copy_source(root, from)?;
                    self.charge_candidate_copy(source, path, pointer_depth(path))?;
                    self.collect_assertions(source, path, false, &mut obligations)?;
                    for (prior, views) in &self.contracts {
                        self.budget.step(root, path)?;
                        if !contains_path(from, prior) {
                            continue;
                        }
                        let suffix = &prior[from.len()..];
                        for view in views {
                            self.budget.add_node(root, path)?;
                            self.budget.add_byte_count(
                                root,
                                path.len().saturating_add(suffix.len()),
                                path,
                            )?;
                            self.budget.step(root, path)?;
                            let rebased = format!("{path}{suffix}");
                            self.budget.charge_work(
                                self.contracts
                                    .get(&rebased)
                                    .map_or(0, Vec::len)
                                    .saturating_add(obligations.len()),
                                root,
                                path,
                            )?;
                            if self.obligation_freshness(&obligations, &rebased, *view)
                                == ObligationFreshness::New
                            {
                                obligations.push((rebased, *view));
                            }
                        }
                    }
                    self.budget.add_bytes(root, path, path)?;
                    self.budget.add_bytes(root, from, path)?;
                    self.budget.add_node(root, path)?;
                    edges.push(CopyEdge {
                        path: path.to_owned(),
                        from: from.to_owned(),
                    });
                    source.clone()
                }
                StageEdit::Expect { .. } => unreachable!("handled constraint edit"),
            };
            let mut assertion_changed = false;
            if let Some(old) = root.at_pointer(path)
                && let Some(tag) = old.tag.as_ref().filter(|tag| tag.supplied)
            {
                // The replacement's assertion remains conjunctive even when
                // the visible destination assertion takes precedence.
                if let Some(view) = tag_schema(self.schemas, &value) {
                    self.add_obligation(&mut obligations, path, view, &value)?;
                }
                self.budget.add_bytes(root, &tag.name, path)?;
                if let Some(binding) = &tag.binding {
                    self.budget.add_bytes(root, binding, path)?;
                }
                assertion_changed = value
                    .tag
                    .as_ref()
                    .is_none_or(|prior| prior.name != tag.name || prior.binding != tag.binding);
                value.tag = Some(tag.clone());
            }
            // Destination assertions and paths are part of the effective
            // summary, including for Copy. Classify before comparing progress
            // or allowing any completion to consume the imported root.
            if matches!(edit, StageEdit::Copy { .. }) {
                self.classify_copy(&mut value, path, &mut active)?;
            } else if assertion_changed {
                self.budget.classify(&mut value, path)?;
            }
            if let Some(old) = root.at_pointer(path) {
                progress |=
                    !same_stage_payload(old, &value, matches!(edit, StageEdit::Copy { .. }));
            } else {
                progress = true;
            }
            progress |= obligations.len() > prior_obligations;
            replacements.push((path.to_owned(), value));
        }
        if !progress {
            return Err(invalid(root, "", "edit batch makes no progress"));
        }
        let mut ancestors = std::collections::BTreeSet::new();
        for path in destinations {
            let mut cursor = path;
            while let Some((parent, _)) = cursor.rsplit_once('/') {
                self.budget.step(root, parent)?;
                if !ancestors.contains(parent) {
                    self.budget.add_node(root, parent)?;
                    ancestors.insert(parent);
                }
                cursor = parent;
            }
        }
        let mut pending = if initial {
            None
        } else {
            let mut pending = PendingDefaults {
                active,
                copied: self
                    .pending_defaults
                    .take()
                    .map_or_else(Vec::new, |prior| prior.copied),
            };
            for edit in edits {
                let path = edit_path(edit);
                if matches!(edit, StageEdit::Replace { .. } | StageEdit::Copy { .. }) {
                    self.budget.charge_work(pending.copied.len(), root, path)?;
                    pending.copied.retain(|prior| !contains_path(path, prior));
                }
                if matches!(edit, StageEdit::Replace { .. }) {
                    PendingDefaults::add_scope(&mut pending.active, path, root, &mut self.budget)?;
                }
            }
            for edge in &edges {
                PendingDefaults::add_scope(
                    &mut pending.copied,
                    &edge.path,
                    root,
                    &mut self.budget,
                )?;
            }
            Some(pending)
        };
        // No data mutation occurs until every import, obligation and pointer
        // has been admitted against the original round snapshot.
        for (path, view) in obligations {
            let views = self.contracts.entry(path).or_default();
            if !views.iter().any(|prior| prior.id() == view.id()) {
                views.push(view);
            }
        }
        for (path, value) in replacements {
            replace_at(root, &path, value);
        }
        // Refresh summaries from the final atomic batch, not intermediate edits.
        // Reverse pointer order visits a descendant before its own ancestors.
        for path in ancestors.into_iter().rev() {
            self.budget.charge_work(pointer_depth(path), root, path)?;
            let node = root.at_pointer_mut(path).expect("admitted ancestor");
            let was_opaque = node.opaque;
            self.budget.classify(node, path)?;
            if was_opaque
                && !node.opaque
                && let Some(pending) = &mut pending
            {
                PendingDefaults::add_scope(&mut pending.active, path, node, &mut self.budget)?;
            }
        }
        if !initial {
            self.pending_defaults = pending;
        }
        Ok(edges)
    }

    // Copy preserves completed values, but opacity belongs to the destination
    // summary/path. A source-opaque descendant that becomes available here has
    // never completed and needs an active exception inside the new copy fence.
    fn classify_copy(
        &mut self,
        node: &mut ResolvedNode<'a>,
        path: &str,
        unblocked: &mut Vec<String>,
    ) -> Result<(), ResolveError> {
        if let Some(tag) = &mut node.tag {
            tag.supplied = true;
        }
        match &mut node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.budget.add_byte_count(
                        child,
                        path.len()
                            .saturating_add(1)
                            .saturating_add(key.len().saturating_mul(2)),
                        path,
                    )?;
                    self.classify_copy(child, &crate::pointer::join(path, key), unblocked)?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    let segment = index.to_string();
                    self.budget.add_byte_count(
                        child,
                        path.len().saturating_add(1).saturating_add(segment.len()),
                        path,
                    )?;
                    self.classify_copy(child, &crate::pointer::join(path, &segment), unblocked)?;
                }
            }
            _ => {}
        }
        let was_opaque = node.opaque;
        self.budget.classify(node, path)?;
        if was_opaque && !node.opaque {
            PendingDefaults::add_scope(unblocked, path, node, &mut self.budget)?;
        }
        Ok(())
    }

    fn completed_copy_source<'n>(
        &mut self,
        root: &'n ResolvedNode<'a>,
        path: &str,
    ) -> Result<&'n ResolvedNode<'a>, ResolveError> {
        let mut node = root;
        self.budget.step(node, path)?;
        if node.opaque {
            return Err(invalid(root, path, "copy source path is deferred"));
        }
        for part in path.split('/').skip(1) {
            let key = crate::pointer::decode_segment(part).expect("admitted escape");
            node = match &node.value {
                ResolvedValue::Mapping(fields) => fields.get(key.as_ref()),
                ResolvedValue::Sequence(items) => {
                    items.get(part.parse::<usize>().expect("admitted index"))
                }
                _ => None,
            }
            .ok_or_else(|| invalid(root, path, "copy source does not exist"))?;
            self.budget.step(node, path)?;
            if node.opaque {
                return Err(invalid(root, path, "copy source path is deferred"));
            }
        }
        Ok(node)
    }

    fn add_obligation(
        &mut self,
        out: &mut Vec<(String, SchemaView<'a>)>,
        path: &str,
        view: SchemaView<'a>,
        node: &ResolvedNode<'_>,
    ) -> Result<ObligationFreshness, ResolveError> {
        self.budget.charge_work(
            self.contracts
                .get(path)
                .map_or(0, Vec::len)
                .saturating_add(out.len()),
            node,
            path,
        )?;
        let freshness = self.obligation_freshness(out, path, view);
        if freshness != ObligationFreshness::New {
            return Ok(freshness);
        }
        self.budget.add_node(node, path)?;
        self.budget.add_bytes(node, path, path)?;
        self.budget.step(node, path)?;
        out.push((path.to_owned(), view));
        Ok(ObligationFreshness::New)
    }

    fn obligation_freshness(
        &self,
        pending: &[(String, SchemaView<'a>)],
        path: &str,
        view: SchemaView<'a>,
    ) -> ObligationFreshness {
        if self
            .contracts
            .get(path)
            .is_some_and(|views| views.iter().any(|prior| prior.id() == view.id()))
        {
            ObligationFreshness::Existing
        } else if pending
            .iter()
            .any(|(prior_path, prior)| prior_path == path && prior.id() == view.id())
        {
            ObligationFreshness::Pending
        } else {
            ObligationFreshness::New
        }
    }
    fn collect_assertions(
        &mut self,
        node: &ResolvedNode<'a>,
        path: &str,
        explicit_only: bool,
        out: &mut Vec<(String, SchemaView<'a>)>,
    ) -> Result<(), ResolveError> {
        self.budget.step(node, path)?;
        if node
            .tag
            .as_ref()
            .is_some_and(|tag| !explicit_only || tag.supplied)
        {
            let view = tag_schema(self.schemas, node).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownTag,
                    node,
                    "unbound staged assertion",
                    None,
                    path,
                )
            })?;
            self.add_obligation(out, path, view, node)?;
        }
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.collect_assertions(
                        child,
                        &crate::pointer::join(path, key),
                        explicit_only,
                        out,
                    )?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (index, child) in items.iter().enumerate() {
                    self.collect_assertions(
                        child,
                        &crate::pointer::join(path, &index.to_string()),
                        explicit_only,
                        out,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

// Edit progress compares exact representation, independently of mathematical
// candidate equality. Only Copy imports provenance; re-importing the same
// semantic payload through Replace must remain a no-op.
fn same_stage_payload(left: &ResolvedNode<'_>, right: &ResolvedNode<'_>, copy: bool) -> bool {
    if left.opaque != right.opaque
        || left.number_class != right.number_class
        || (copy && left.origin != right.origin)
    {
        return false;
    }
    match (&left.tag, &right.tag) {
        (Some(left), Some(right))
            if left.name == right.name
                && left.binding == right.binding
                && left.supplied == right.supplied
                && (!copy || left.origin == right.origin) => {}
        (None, None) => {}
        _ => return false,
    }
    match (&left.value, &right.value) {
        (ResolvedValue::Null, ResolvedValue::Null) => true,
        (ResolvedValue::Bool(left), ResolvedValue::Bool(right)) => left == right,
        (ResolvedValue::Number(left), ResolvedValue::Number(right)) => {
            left.as_str() == right.as_str()
        }
        (ResolvedValue::String(left), ResolvedValue::String(right)) => left == right,
        (ResolvedValue::Sequence(left), ResolvedValue::Sequence(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same_stage_payload(left, right, copy))
        }
        (ResolvedValue::Mapping(left), ResolvedValue::Mapping(right)) => {
            left.len() == right.len()
                && left.iter().zip(right).all(|((key, left), (other, right))| {
                    key == other && same_stage_payload(left, right, copy)
                })
        }
        _ => false,
    }
}

fn invalid(root: &ResolvedNode<'_>, path: &str, message: &str) -> ResolveError {
    failure(
        ResolveErrorKind::InvalidStage,
        root.at_pointer(path).unwrap_or(root),
        message,
        None,
        path,
    )
}
fn edit_path(edit: &StageEdit) -> &str {
    match edit {
        StageEdit::Replace { path, .. }
        | StageEdit::Copy { path, .. }
        | StageEdit::Expect { path, .. } => path,
    }
}
fn pointer_depth(path: &str) -> usize {
    path.bytes().filter(|byte| *byte == b'/').count()
}
fn contains_path(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|tail| tail.starts_with('/'))
}
fn overlaps(a: &str, b: &str) -> bool {
    contains_path(a, b) || contains_path(b, a)
}
fn validate_pointer(root: &ResolvedNode<'_>, path: &str) -> Result<(), ResolveError> {
    if path.is_empty() {
        return Ok(());
    }
    let Some(parts) = path.strip_prefix('/') else {
        return Err(invalid(root, path, "invalid instance pointer"));
    };
    if parts
        .split('/')
        .any(|part| crate::pointer::decode_segment(part).is_none())
    {
        return Err(invalid(root, path, "invalid pointer escape"));
    }
    let mut node = root;
    for part in parts.split('/') {
        let key = crate::pointer::decode_segment(part).expect("validated pointer escape");
        match &node.value {
            ResolvedValue::Mapping(fields) => match fields.get(key.as_ref()) {
                Some(child) => node = child,
                None => break,
            },
            ResolvedValue::Sequence(items) => {
                if part != "0" && !matches!(part.as_bytes().first(), Some(b'1'..=b'9')) {
                    return Err(invalid(root, path, "noncanonical sequence index"));
                }
                let index = part
                    .parse::<usize>()
                    .map_err(|_| invalid(root, path, "invalid sequence index"))?;
                match items.get(index) {
                    Some(child) => node = child,
                    None => break,
                }
            }
            _ => break,
        }
    }
    Ok(())
}
fn validate_destination(root: &ResolvedNode<'_>, path: &str) -> Result<(), ResolveError> {
    if path.is_empty() {
        return Ok(());
    }
    let (parent, part) = path.rsplit_once('/').expect("validated pointer");
    let key = crate::pointer::decode_segment(part).expect("validated pointer escape");
    let node = root
        .at_pointer(parent)
        .ok_or_else(|| invalid(root, path, "edit parent does not exist"))?;
    match &node.value {
        ResolvedValue::Mapping(_) if crate::names::valid_key(&key) => Ok(()),
        ResolvedValue::Mapping(_) => Err(failure(
            ResolveErrorKind::InvalidKey,
            node,
            "staged mapping key violates the DAML key grammar",
            None,
            path,
        )),
        ResolvedValue::Sequence(items)
            if part
                .parse::<usize>()
                .ok()
                .is_some_and(|index| index < items.len()) =>
        {
            Ok(())
        }
        _ => Err(invalid(
            root,
            path,
            "edit requires an existing mapping or in-range sequence parent",
        )),
    }
}
fn replace_at<'a>(root: &mut ResolvedNode<'a>, path: &str, value: ResolvedNode<'a>) {
    if path.is_empty() {
        *root = value;
        return;
    }
    let mut parts = path
        .strip_prefix('/')
        .expect("admitted pointer")
        .split('/')
        .peekable();
    let mut node = root;
    while let Some(part) = parts.next() {
        let key = crate::pointer::decode_segment(part).expect("admitted escape");
        if parts.peek().is_none() {
            match &mut node.value {
                ResolvedValue::Mapping(fields) => {
                    fields.insert(Cow::Owned(key.into_owned()), value);
                }
                ResolvedValue::Sequence(items) => {
                    items[part.parse::<usize>().expect("admitted index")] = value
                }
                _ => unreachable!("admitted destination"),
            }
            return;
        }
        node = match &mut node.value {
            ResolvedValue::Mapping(fields) => {
                fields.get_mut(key.as_ref()).expect("admitted parent")
            }
            ResolvedValue::Sequence(items) => {
                &mut items[part.parse::<usize>().expect("admitted index")]
            }
            _ => unreachable!("admitted parent"),
        };
    }
}
fn own_semantic_node<'a>(node: ResolvedNode<'_>) -> ResolvedNode<'a> {
    let value = match node.value {
        ResolvedValue::Null => ResolvedValue::Null,
        ResolvedValue::Bool(value) => ResolvedValue::Bool(value),
        ResolvedValue::Number(value) => ResolvedValue::Number(Cow::Owned(value.into_owned())),
        ResolvedValue::String(value) => ResolvedValue::String(Cow::Owned(value.into_owned())),
        ResolvedValue::Sequence(items) => {
            ResolvedValue::Sequence(items.into_iter().map(own_semantic_node).collect())
        }
        ResolvedValue::Mapping(fields) => ResolvedValue::Mapping(
            fields
                .into_iter()
                .map(|(key, child)| (Cow::Owned(key.into_owned()), own_semantic_node(child)))
                .collect(),
        ),
    };
    ResolvedNode {
        value,
        tag: node.tag.map(|tag| ResolvedTag {
            name: Cow::Owned(tag.name.into_owned()),
            binding: tag.binding.map(|binding| Cow::Owned(binding.into_owned())),
            origin: Origin::Semantic,
            supplied: true,
        }),
        origin: Origin::Semantic,
        opaque: node.opaque,
        number_class: node.number_class,
    }
}
