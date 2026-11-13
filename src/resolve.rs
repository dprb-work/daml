use crate::{
    Document, ExplainError, Explanation, ExplanationConstraint, Node, Origin, ResolutionLimits,
    ResolveError, ResolveErrorKind, ResolvedDocument, ResolvedNode, ResolvedValue, SchemaLocation,
    Schemas, SemanticNode, SemanticValue, Value,
    explain::{CaptureError, capture_evaluation, reserve, select_pointer},
    json_backend::equal_resolved,
    resolved::{ErrorOrigin, ResolvedTag, check_number_limits, checked_number},
    schema::{SchemaId, SchemaView},
};
use serde_json::Value as JsonValue;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
};

#[path = "stage.rs"]
pub(crate) mod stage;

struct Resolver<'a, 's> {
    schemas: &'a Schemas,
    budget: Budget<'s>,
    search: CompletionSearch<'a>,
    contracts: BTreeMap<String, Vec<SchemaView<'a>>>,
    // None is initial completion. Continuations scope fresh/resumed preset work
    // while retaining the completed role of copied subtrees across rounds.
    pending_defaults: Option<stage::PendingDefaults>,
}

// Keep text work lazy so an early contradiction need not inspect later names.
// The budget is borrowed only by next(), leaving recursive consumers free to use it.
struct PropertyNames<'a, I> {
    names: I,
    seen: HashSet<&'a str>,
}

impl<'a, I: Iterator<Item = &'a str>> PropertyNames<'a, I> {
    fn next(
        &mut self,
        budget: &mut Budget<'_>,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<Option<&'a str>, ResolveError> {
        for name in self.names.by_ref() {
            // Reserve hashing and a spelling comparison before deduplication.
            budget.charge_work(
                name.len().div_ceil(64).saturating_add(1).saturating_mul(2),
                node,
                path,
            )?;
            if self.seen.insert(name) {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }
}

enum CandidateSource<'a> {
    Document(&'a Document),
    Semantic(&'a SemanticNode),
    Resolved(ResolvedNode<'a>),
}

// Input resolution requires one semantic result; partial defaults need only
// one valid completion so omitted-data ambiguity remains a runtime decision.
#[derive(Clone, Copy)]
enum CandidateRequirement {
    Unique,
    Feasible,
}

#[derive(Clone, Copy)]
enum TagAdmission {
    // Constraint-only validation does not retain a resolved snapshot.
    Validate,
    // A completed result checks constraints and retains canonical bindings.
    Resolve,
    // A staged candidate pins bindings before its constraints can be checked.
    PinCandidate,
}

struct DefaultLayers {
    tag_owned: HashSet<SchemaId>,
    tag_branches: HashSet<SchemaId>,
    branch_fallbacks: HashSet<SchemaId>,
}

impl Schemas {
    /// Completes declared defaults and validates every explicit tag, even under open fields.
    ///
    /// # Errors
    ///
    /// Returns unknown-tag, schema, ambiguous-default, numeric, or resource failures.
    pub fn resolve<'a>(
        &'a self,
        document: &'a Document,
    ) -> Result<ResolvedDocument<'a>, ResolveError> {
        self.resolve_with_limits(document, None, ResolutionLimits::default())
    }

    /// Adds the named root type's constraints to all explicit tag assertions.
    ///
    /// # Errors
    ///
    /// Returns resolution failures, including an unknown expected type.
    pub fn resolve_as<'a>(
        &'a self,
        document: &'a Document,
        expected: &str,
    ) -> Result<ResolvedDocument<'a>, ResolveError> {
        self.resolve_with_limits(document, Some(expected), ResolutionLimits::default())
    }

    /// Resolves with explicit cumulative materialization and schema-work budgets.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::resolve`], stopping when a budget is exhausted.
    pub fn resolve_with_limits<'a>(
        &'a self,
        document: &'a Document,
        expected: Option<&str>,
        limits: ResolutionLimits,
    ) -> Result<ResolvedDocument<'a>, ResolveError> {
        Resolver {
            schemas: self,
            budget: Budget {
                limits,
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: None,
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        }
        .run(document, expected)
    }

    /// Admits the entire document, then explains one RFC 6901 instance pointer.
    ///
    /// Both a root assertion and an expected contract remain obligations when
    /// supplied together. Returned values borrow the document and this registry;
    /// constraints retain admitted schemas rather than expanding references.
    /// Ordinary resolution does not collect or allocate this evaluation trace.
    ///
    /// # Errors
    ///
    /// Rejects missing root contracts, ordinary resolution failures, malformed or
    /// absent pointers, and exhaustion of the cumulative resolution/trace budgets.
    pub fn explain<'a>(
        &'a self,
        document: &'a Document,
        expected: Option<&str>,
        pointer: &str,
    ) -> Result<Explanation<'a>, ExplainError> {
        if expected.is_none() && document.tag(document.root()).is_none() {
            return Err(ExplainError::MissingRootContract);
        }
        let mut resolver = Resolver {
            schemas: self,
            budget: Budget {
                limits: ResolutionLimits::default(),
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: None,
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        };
        let resolved = resolver.run(document, expected)?;
        let decoded_bytes = pointer
            .split('/')
            .filter(|part| part.contains('~'))
            .fold(0usize, |bytes, part| bytes.saturating_add(part.len()));
        resolver.budget.add_byte_count(
            resolved.root(),
            pointer
                .len()
                .saturating_add(decoded_bytes.saturating_mul(2)),
            "",
        )?;
        resolver.budget.charge_work(
            pointer.len().div_ceil(64).saturating_mul(3),
            resolved.root(),
            "",
        )?;
        select_pointer(resolved.root(), pointer)?;
        let mut constraints = Vec::new();
        let mut seen = HashSet::new();
        if let Some(expected) = expected {
            let schema = resolver.expected_view(resolved.root(), expected)?;
            resolver.explain_contract(
                resolved.root(),
                schema,
                "",
                pointer,
                &mut constraints,
                &mut seen,
            )?;
        }
        let mut node = resolved.root();
        let mut path_end = 0;
        if let Some(schema) = tag_schema(self, node) {
            resolver.explain_contract(node, schema, "", pointer, &mut constraints, &mut seen)?;
        }
        if !pointer.is_empty() {
            for part in pointer[1..].split('/') {
                path_end += part.len() + 1;
                let key = crate::pointer::decode_segment(part)
                    .expect("the selected pointer has valid escapes");
                node = match &node.value {
                    ResolvedValue::Mapping(fields) => fields
                        .get(key.as_ref())
                        .expect("the selected mapping member exists"),
                    ResolvedValue::Sequence(items) => {
                        &items[key
                            .parse::<usize>()
                            .expect("the selected sequence index is canonical")]
                    }
                    _ => unreachable!("the selected pointer traverses only containers"),
                };
                if let Some(schema) = tag_schema(self, node) {
                    resolver.explain_contract(
                        node,
                        schema,
                        &pointer[..path_end],
                        pointer,
                        &mut constraints,
                        &mut seen,
                    )?;
                }
            }
        }
        Ok(Explanation {
            document: resolved,
            path: pointer.to_owned(),
            constraints,
        })
    }

    /// Completes and validates explicit in-memory semantic input without source conversion.
    ///
    /// # Errors
    ///
    /// Returns grammar, unknown-tag, schema, ambiguity, numeric, or resource failures.
    pub fn resolve_semantic<'a>(
        &'a self,
        node: &'a SemanticNode,
        expected: Option<&str>,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        self.resolve_semantic_with_limits(node, expected, ResolutionLimits::default())
    }

    /// Admits semantic input with cumulative materialization and schema-work budgets.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::resolve_semantic`], bounded by `limits`.
    pub fn resolve_semantic_with_limits<'a>(
        &'a self,
        node: &'a SemanticNode,
        expected: Option<&str>,
        limits: ResolutionLimits,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        Resolver {
            schemas: self,
            budget: Budget {
                limits,
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: None,
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        }
        .run_semantic(node, expected)
    }

    /// Validates supplied semantic data and assertions without applying completion.
    ///
    /// Literal JSON mapping keys are accepted even when DAML source cannot represent
    /// them. Success does not guarantee serialization. The input is not mutated.
    ///
    /// # Errors
    ///
    /// Returns tag, binding, expected-type, schema, numeric, or resource failures.
    pub fn validate_semantic(
        &self,
        node: &SemanticNode,
        expected: Option<&str>,
    ) -> Result<(), ResolveError> {
        let mut resolver = Resolver {
            schemas: self,
            budget: Budget {
                limits: ResolutionLimits::default(),
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: None,
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        };
        let mut root = from_semantic_with_keys(node, 0, "", &mut resolver.budget, false)?;
        resolver.known_tags(&root, "")?;
        if let Some(expected) = expected {
            let schema = resolver.expected_view(&root, expected)?;
            Resolver::check_node(&root, &schema, "", &mut resolver.budget)?;
        }
        resolver.admit_tags(&mut root, "", 0, TagAdmission::Validate)
    }

    pub(crate) fn complete_partial_default<'a>(
        &'a self,
        expected: &str,
        node: ResolvedNode<'a>,
        limits: &mut ResolutionLimits,
        fixed_choices: &[(String, usize)],
        candidate_origin: &str,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        let view = self.type_schema(expected).ok_or_else(|| {
            failure(
                ResolveErrorKind::UnknownExpectedType,
                &node,
                format!("no schema is bound to expected type {expected:?}"),
                None,
                "",
            )
        })?;
        let mut root_views = Vec::new();
        if let Some(tag_view) = tag_schema(self, &node) {
            root_views.push(tag_view);
        }
        root_views.push(view);
        let remaining = *limits;
        let mut resolver = Resolver {
            schemas: self,
            budget: Budget {
                limits: remaining,
                nodes: 0,
                bytes: 0,
                steps: 0,
                stage: None,
            },
            search: CompletionSearch::default(),
            contracts: BTreeMap::new(),
            pending_defaults: None,
        };
        let result = (|| {
            resolver.budget.step(&node, "")?;
            let origin = self.binding_schema(candidate_origin).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownExpectedType,
                    &node,
                    "native default origin references an unknown schema",
                    None,
                    "",
                )
            })?;
            if origin.value().get("default").is_none() {
                return Err(failure(
                    ResolveErrorKind::SchemaViolation,
                    &node,
                    "native default origin has no own default",
                    Some(origin.location().clone()),
                    "",
                ));
            }
            resolver.search.suppressed_root_default = Some(origin.id());
            for (reference, index) in fixed_choices {
                resolver.budget.step(&node, "")?;
                let union = self.binding_schema(reference).ok_or_else(|| {
                    failure(
                        ResolveErrorKind::UnknownExpectedType,
                        &node,
                        "native default branch references an unknown schema",
                        None,
                        "",
                    )
                })?;
                let branch = union.array_child("anyOf", *index).ok_or_else(|| {
                    failure(
                        ResolveErrorKind::SchemaViolation,
                        &node,
                        "native default branch index does not identify an anyOf arm",
                        Some(union.location().clone()),
                        "",
                    )
                })?;
                if let Some(prior) = resolver.search.fixed_root_any_of.get(&union.id()) {
                    if prior.id() != branch.id() {
                        return Err(failure(
                            ResolveErrorKind::SchemaViolation,
                            &node,
                            "native default branch assignments conflict",
                            Some(union.location().clone()),
                            "",
                        ));
                    }
                } else {
                    resolver.budget.add_node(&node, "")?;
                    resolver.budget.add_byte_count(
                        &node,
                        std::mem::size_of::<(SchemaId, SchemaView<'_>)>(),
                        "",
                    )?;
                    resolver.search.fixed_root_any_of.insert(union.id(), branch);
                }
            }
            resolver.known_tags(&node, "")?;
            resolver
                .select_candidates(
                    &root_views,
                    None,
                    CandidateSource::Resolved(node),
                    CandidateRequirement::Feasible,
                    &[],
                )
                .map(|result| result.expect("nonstaged candidates cannot reject").root)
        })();
        limits.max_nodes = remaining.max_nodes.saturating_sub(resolver.budget.nodes);
        limits.max_bytes = remaining.max_bytes.saturating_sub(resolver.budget.bytes);
        limits.max_steps = remaining.max_steps.saturating_sub(resolver.budget.steps);
        result
    }
}

impl<'a> Resolver<'a, '_> {
    fn explain_contract(
        &mut self,
        node: &ResolvedNode<'a>,
        schema: SchemaView<'a>,
        path: &str,
        target: &str,
        constraints: &mut Vec<ExplanationConstraint<'a>>,
        seen: &mut HashSet<(usize, SchemaId)>,
    ) -> Result<(), ResolveError> {
        let identity = (node as *const ResolvedNode<'_> as usize, schema.id());
        if seen.contains(&identity) {
            return Ok(());
        }
        let mut remaining = ResolutionLimits {
            max_nodes: self
                .budget
                .limits
                .max_nodes
                .saturating_sub(self.budget.nodes),
            max_bytes: self
                .budget
                .limits
                .max_bytes
                .saturating_sub(self.budget.bytes),
            max_steps: self
                .budget
                .limits
                .max_steps
                .saturating_sub(self.budget.steps),
        };
        let estimate = schema.explanation_resources(node, remaining);
        reserve(
            &mut remaining,
            estimate.max_nodes,
            estimate.max_bytes,
            estimate.max_steps,
        )
        .map_err(|_| {
            failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "eager explanation evaluation exceeds cumulative resolution bounds",
                Some(schema.explanation_location().clone()),
                path,
            )
        })?;
        let record_bytes = std::mem::size_of::<ExplanationConstraint<'_>>()
            + std::mem::size_of::<(usize, SchemaId)>();
        let result = (|| {
            // Always retain the actual entry contract, including a boolean or
            // pure reference whose optimized evaluation may have no own unit.
            reserve(
                &mut remaining,
                1,
                path.len().saturating_add(record_bytes),
                1,
            )?;
            seen.insert(identity);
            constraints.push(ExplanationConstraint {
                instance_path: path.to_owned(),
                location: schema.explanation_location(),
                schema: schema.value(),
            });
            let evaluation = schema.evaluate(node);
            if !evaluation.is_valid() {
                return Err(CaptureError::Invalid(
                    "schema evaluation rejected a previously admitted value".into(),
                ));
            }
            let target_suffix = target
                .strip_prefix(path)
                .expect("only selected ancestors are independently explained");
            capture_evaluation(&evaluation, &mut remaining, &mut |mut unit, remaining| {
                let Some(relative) = schema.evaluation_instance_path(&unit.instance_location)
                else {
                    // The private dispatch object's root/properties wrappers
                    // are not contracts on user data, but their children are.
                    return Ok(true);
                };
                if relative != target_suffix
                    && !target_suffix
                        .strip_prefix(relative)
                        .is_some_and(|suffix| suffix.starts_with('/'))
                {
                    return Ok(false);
                }
                if unit.schema_location.contains('%') {
                    reserve(
                        remaining,
                        0,
                        unit.schema_location.len(),
                        unit.schema_location.len().div_ceil(64),
                    )?;
                }
                let Some((applied, descend)) = schema.evaluation_schema(&unit.schema_location)
                else {
                    return Ok(true);
                };
                let Some(applied) = applied else {
                    return Ok(descend);
                };
                if relative.contains('~') {
                    reserve(remaining, 0, relative.len(), relative.len().div_ceil(64))?;
                }
                let instance = node.at_pointer(relative).ok_or_else(|| {
                    CaptureError::Invalid("evaluation identifies an absent instance".into())
                })?;
                let identity = (instance as *const ResolvedNode<'_> as usize, applied.id());
                if !seen.contains(&identity) {
                    reserve(remaining, 1, path.len().saturating_add(record_bytes), 1)?;
                    // Reuse the already charged instance-location allocation;
                    // remove only the engine's private singleton dispatch key.
                    let dispatch_bytes = unit.instance_location.len() - relative.len();
                    unit.instance_location.drain(..dispatch_bytes);
                    unit.instance_location.insert_str(0, path);
                    seen.insert(identity);
                    constraints.push(ExplanationConstraint {
                        instance_path: unit.instance_location,
                        location: applied.explanation_location(),
                        schema: applied.value(),
                    });
                }
                Ok(descend)
            })
        })();
        self.budget.nodes = self.budget.limits.max_nodes - remaining.max_nodes;
        self.budget.bytes = self.budget.limits.max_bytes - remaining.max_bytes;
        self.budget.steps = self.budget.limits.max_steps - remaining.max_steps;
        result.map_err(|error| {
            let kind = match error {
                CaptureError::Limit => ResolveErrorKind::LimitExceeded,
                CaptureError::Invalid(_) => ResolveErrorKind::SchemaViolation,
            };
            failure(
                kind,
                node,
                error.to_string(),
                Some(schema.explanation_location().clone()),
                path,
            )
        })
    }

    fn same_values(&self, left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
        if self.budget.stage.is_some() {
            same_json(left, right)
        } else {
            equal_resolved(left, right, true)
        }
    }

    fn run(
        &mut self,
        document: &'a Document,
        expected: Option<&str>,
    ) -> Result<ResolvedDocument<'a>, ResolveError> {
        let root = from_input(document, document.root(), 0, "", &mut self.budget)?;
        self.known_tags(&root, "")?;
        let root_views = self.root_views(&root, expected)?;
        let root = self.select_candidates(
            &root_views,
            Some(root),
            CandidateSource::Document(document),
            CandidateRequirement::Unique,
            &[],
        )?;
        let root = root.expect("nonstaged candidates cannot reject").root;
        Ok(ResolvedDocument { document, root })
    }

    fn run_semantic(
        &mut self,
        node: &'a SemanticNode,
        expected: Option<&str>,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        let root = from_semantic(node, 0, "", &mut self.budget)?;
        self.known_tags(&root, "")?;
        let root_views = self.root_views(&root, expected)?;
        self.select_candidates(
            &root_views,
            Some(root),
            CandidateSource::Semantic(node),
            CandidateRequirement::Unique,
            &[],
        )
        .map(|result| result.expect("nonstaged candidates cannot reject").root)
    }

    fn root_views(
        &self,
        root: &ResolvedNode<'_>,
        expected: Option<&str>,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut views = Vec::new();
        if let Some(view) = tag_schema(self.schemas, root) {
            views.push(view);
        }
        if let Some(expected) = expected {
            views.push(self.expected_view(root, expected)?);
        }
        Ok(views)
    }

    fn expected_view(
        &self,
        root: &ResolvedNode<'_>,
        expected: &str,
    ) -> Result<SchemaView<'a>, ResolveError> {
        self.schemas.type_schema(expected).ok_or_else(|| {
            failure(
                ResolveErrorKind::UnknownExpectedType,
                root,
                format!("no schema is bound to expected type {expected:?}"),
                None,
                "",
            )
        })
    }

    fn select_candidates(
        &mut self,
        root_views: &[SchemaView<'a>],
        mut initial: Option<ResolvedNode<'a>>,
        source: CandidateSource<'a>,
        requirement: CandidateRequirement,
        initial_edits: &[stage::StageEdit],
    ) -> Result<Option<stage::StagedResolution<'a>>, ResolveError> {
        let mut selected: Option<stage::StagedResolution<'a>> = None;
        let mut first_error = None;
        let mut rejected = false;
        loop {
            self.search.start();
            self.contracts.clear();
            self.pending_defaults = None;
            let mut root = match initial.take() {
                Some(root) => root,
                None => match &source {
                    CandidateSource::Document(document) => {
                        from_input(document, document.root(), 0, "", &mut self.budget)?
                    }
                    CandidateSource::Semantic(node) => {
                        from_semantic(node, 0, "", &mut self.budget)?
                    }
                    CandidateSource::Resolved(node) => {
                        self.charge_candidate_copy(node, "", 0)?;
                        node.clone()
                    }
                },
            };
            let candidate = if self.budget.stage.is_some() {
                self.known_tags(&root, "")?;
                self.apply_edits(&mut root, initial_edits, true)?;
                self.stage_candidate(root, root_views)
            } else {
                self.complete(root, root_views, "", CompletionLevel::existing(0))
                    .and_then(|root| self.finish_candidate(root_views, root))
                    .map(|root| {
                        Some(stage::StagedResolution {
                            root,
                            meaning: None,
                        })
                    })
            };
            match candidate {
                Ok(Some(candidate)) => {
                    if let Some(error) = self.search.conflict.take()
                        && matches!(requirement, CandidateRequirement::Unique)
                    {
                        return Err(error);
                    }
                    if matches!(requirement, CandidateRequirement::Feasible) {
                        return Ok(Some(candidate));
                    }
                    if let Some(prior) = &selected {
                        if !self.same_values(&prior.root, &candidate.root)
                            || prior.meaning != candidate.meaning
                        {
                            let location = self
                                .search
                                .branches
                                .values()
                                .flat_map(|branches| branches.views.values())
                                .next()
                                .map(|view| view.location());
                            return Err(match location {
                                Some(location) => {
                                    ambiguous_completion(&prior.root, &candidate.root, "", location)
                                }
                                None => failure(
                                    ResolveErrorKind::AmbiguousDefault,
                                    &candidate.root,
                                    "valid candidates have different admission meanings",
                                    None,
                                    "",
                                ),
                            });
                        }
                    } else {
                        selected = Some(candidate);
                    }
                }
                Ok(None) => rejected = true,
                Err(error)
                    if self
                        .budget
                        .stage
                        .as_ref()
                        .is_some_and(|stage| stage.failed())
                        || matches!(
                            error.kind,
                            ResolveErrorKind::LimitExceeded
                                | ResolveErrorKind::NumericLimit
                                | ResolveErrorKind::InvalidStage
                        ) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            if !self.search.advance() {
                break;
            }
        }
        if selected.is_some() {
            Ok(selected)
        } else if let Some(error) = first_error {
            Err(error)
        } else {
            assert!(
                rejected,
                "every unsuccessful candidate records a failure or rejection"
            );
            Ok(None)
        }
    }
    fn finish_candidate(
        &mut self,
        root_views: &[SchemaView<'a>],
        mut root: ResolvedNode<'a>,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        let asserted = root
            .tag()
            .and_then(|_| tag_schema(self.schemas, &root))
            .map(|view| view.id());
        self.admit_tags(&mut root, "", 0, TagAdmission::Resolve)?;
        for view in root_views {
            if Some(view.id()) != asserted {
                Self::check_node(&root, view, "", &mut self.budget)?;
            }
        }
        // Every selected branch must hold; enclosing validators still enforce
        // oneOf's exact-one semantics.
        for (path, branches) in &self.search.branches {
            if let Some(node) = root.at_pointer(path) {
                for view in branches.views.values() {
                    Self::check_node(node, view, path, &mut self.budget)?;
                }
            } else if self.budget.stage.is_some() && branches.materialized {
                return Err(failure(
                    ResolveErrorKind::SchemaViolation,
                    &root,
                    "a selected-branch obligated value is missing",
                    None,
                    path,
                ));
            }
        }
        for (id, branch) in &self.search.fixed_root_any_of {
            self.budget.step(&root, "")?;
            if !self
                .search
                .branches
                .get("")
                .and_then(|branches| branches.views.get(&(*id, "anyOf")))
                .is_some_and(|selected| selected.id() == branch.id())
            {
                return Err(failure(
                    ResolveErrorKind::SchemaViolation,
                    &root,
                    "a required native default branch was not selected",
                    Some(branch.location().clone()),
                    "",
                ));
            }
        }
        if let Some(origin) = self.search.suppressed_root_default {
            let selected = self
                .search
                .branches
                .get("")
                .into_iter()
                .flat_map(|branches| branches.views.values().copied())
                .collect::<Vec<_>>();
            let reachable =
                self.schema_conjunction(root_views.iter().copied().chain(selected), &root, "")?;
            if !reachable.iter().any(|schema| schema.id() == origin) {
                return Err(failure(
                    ResolveErrorKind::SchemaViolation,
                    &root,
                    "native default origin is outside the selected root context",
                    None,
                    "",
                ));
            }
        }
        Ok(root)
    }
    fn charge_candidate_copy(
        &mut self,
        node: &ResolvedNode<'_>,
        path: &str,
        depth: usize,
    ) -> Result<(), ResolveError> {
        self.budget.add_node(node, path)?;
        if depth > 128
            || (depth == 128
                && matches!(
                    node.value,
                    ResolvedValue::Sequence(_) | ResolvedValue::Mapping(_)
                ))
        {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "input exceeds depth 128",
                None,
                path,
            ));
        }
        if let Some(tag) = &node.tag {
            self.budget.add_bytes(node, &tag.name, path)?;
            if let Some(binding) = &tag.binding {
                self.budget.add_bytes(node, binding, path)?;
            }
        }
        match &node.value {
            ResolvedValue::Number(number) => self.budget.add_bytes(node, number.as_str(), path)?,
            ResolvedValue::String(text) => self.budget.add_bytes(node, text, path)?,
            ResolvedValue::Temporal(value) => self.budget.add_bytes(node, value.as_str(), path)?,
            ResolvedValue::Sequence(items) => {
                for (index, child) in items.iter().enumerate() {
                    let child_path = crate::pointer::join(path, &index.to_string());
                    self.charge_candidate_copy(child, &child_path, depth + 1)?;
                }
            }
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    let child_path = crate::pointer::join(path, key);
                    self.budget.add_bytes(node, key, &child_path)?;
                    self.charge_candidate_copy(child, &child_path, depth + 1)?;
                }
            }
            ResolvedValue::Null | ResolvedValue::Bool(_) => {}
        }
        Ok(())
    }
}

fn tag_schema<'a>(schemas: &'a Schemas, node: &ResolvedNode<'_>) -> Option<SchemaView<'a>> {
    node.tag.as_ref().and_then(|tag| match &tag.binding {
        Some(binding) if matches!(tag.origin, Origin::Semantic) => {
            schemas.semantic_binding_schema(binding)
        }
        Some(binding) => schemas.binding_schema(binding),
        None => schemas.type_schema(&tag.name),
    })
}
fn same_tag_identity(schemas: &Schemas, left: &ResolvedTag<'_>, right: &ResolvedTag<'_>) -> bool {
    let left_name = left.binding.as_deref().unwrap_or(left.name.as_ref());
    let right_name = right.binding.as_deref().unwrap_or(right.name.as_ref());
    if left_name == right_name {
        return true;
    }
    // A staged snapshot may already carry an actual admissible contract ID
    // while a newly discovered default still names its local alias wrapper.
    // Keep the original spelling comparison for two unpinned native assertions.
    if !matches!(left.origin, Origin::Semantic)
        && !matches!(right.origin, Origin::Semantic)
        && ![left, right].iter().any(|tag| {
            tag.binding
                .as_deref()
                .is_some_and(|binding| schemas.semantic_binding_schema(binding).is_some())
        })
    {
        return false;
    }
    schemas
        .binding_identity(left_name)
        .zip(schemas.binding_identity(right_name))
        .is_some_and(|(left, right)| left == right)
}
impl<'a> Resolver<'a, '_> {
    fn default_layers(
        &mut self,
        node: &ResolvedNode<'a>,
        path: &str,
    ) -> Result<DefaultLayers, ResolveError> {
        let mut owned = HashSet::new();
        let mut tag_branches = HashSet::new();
        let mut pending = tag_schema(self.schemas, node)
            .into_iter()
            .map(|view| (view, false))
            .collect::<Vec<_>>();
        while let Some((view, branch_layer)) = pending.pop() {
            if branch_layer {
                if owned.contains(&view.id()) || !tag_branches.insert(view.id()) {
                    continue;
                }
            } else if !owned.insert(view.id()) {
                tag_branches.remove(&view.id());
                continue;
            } else {
                tag_branches.remove(&view.id());
            }
            self.budget.step(node, path)?;
            if let Some(reference) = view.referenced() {
                pending.push((reference, branch_layer));
            }
            pending.extend(
                view.array_children("allOf")
                    .map(|child| (child, branch_layer)),
            );
            if let Some(branches) = self.search.branches.get(path) {
                pending.extend(
                    branches
                        .views
                        .iter()
                        .filter(|((parent, _), _)| *parent == view.id())
                        .map(|(_, branch)| (*branch, true)),
                );
            }
        }
        tag_branches.retain(|id| !owned.contains(id));
        let mut branch_fallbacks = HashSet::new();
        let mut pending = self
            .search
            .branches
            .get(path)
            .into_iter()
            .flat_map(|branches| branches.views.values().copied())
            .collect::<Vec<_>>();
        while let Some(view) = pending.pop() {
            if owned.contains(&view.id())
                || tag_branches.contains(&view.id())
                || !branch_fallbacks.insert(view.id())
            {
                continue;
            }
            self.budget.step(node, path)?;
            pending.extend(view.referenced());
            pending.extend(view.array_children("allOf"));
        }
        Ok(DefaultLayers {
            tag_owned: owned,
            tag_branches,
            branch_fallbacks,
        })
    }
}
impl<'a> Resolver<'a, '_> {
    fn complete(
        &mut self,
        mut node: ResolvedNode<'a>,
        schemas: &[SchemaView<'a>],
        path: &str,
        level: CompletionLevel,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        let depth = level.depth;
        let mut level = level;
        if level.revisit
            && let Some(pending) = &self.pending_defaults
        {
            level.apply_node_defaults &=
                pending.scope(path, &node, &mut self.budget)? != stage::DefaultScope::Skip;
        }
        self.budget.step(&node, path)?;
        if depth >= 128
            && matches!(
                node.value,
                ResolvedValue::Mapping(_) | ResolvedValue::Sequence(_)
            )
        {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                &node,
                "resolved collection nesting exceeds 128",
                None,
                path,
            ));
        }
        let mut schemas = schemas.to_vec();
        if node.opaque {
            return Ok(node);
        }
        if let Some(contracts) = self.contracts.get(path) {
            schemas.extend_from_slice(contracts);
        }
        schemas = self.complete_node_defaults(&mut node, schemas, path, level)?;

        if let ResolvedValue::Mapping(fields) = &mut node.value {
            for (key, child) in fields.iter_mut() {
                let child_path = crate::pointer::join(path, key);
                let child_schemas = self.property_schemas(&schemas, key, child, &child_path)?;
                let placeholder = ResolvedNode {
                    value: ResolvedValue::Null,
                    tag: None,
                    origin: child.origin,
                    opaque: false,
                    number_class: 0,
                };
                let owned = std::mem::replace(child, placeholder);
                *child = self.complete(owned, &child_schemas, &child_path, level.child())?;
            }
        }
        if matches!(node.value, ResolvedValue::Mapping(_)) {
            let mut names = self.property_names(&schemas, &node, path)?;
            let mut supplied_lookup_work = self.supplied_property_lookup_work(&node, path)?;
            while let Some(key) = names.next(&mut self.budget, &node, path)? {
                self.budget.charge_work(supplied_lookup_work, &node, path)?;
                if node.get(key).is_some() {
                    continue;
                }
                let child_schemas = self.property_schemas(&schemas, key, &node, path)?;
                let child_path = crate::pointer::join(path, key);
                let required = self.requires_property(&schemas, key, &node, path)?;
                if let Some(child) = self.complete_missing(
                    &child_schemas,
                    &child_path,
                    depth + 1,
                    node.origin,
                    required,
                )? {
                    self.budget.add_bytes(&node, key, &child_path)?;
                    if let ResolvedValue::Mapping(fields) = &mut node.value {
                        fields.insert(Cow::Borrowed(key), child);
                        supplied_lookup_work = supplied_lookup_work
                            .saturating_add(key.len().div_ceil(64).saturating_add(1));
                    }
                }
            }
        } else if let ResolvedValue::Sequence(items) = &mut node.value {
            for (index, child) in items.iter_mut().enumerate() {
                let item_schemas: Vec<_> = schemas
                    .iter()
                    .filter_map(|schema| schema.item(index))
                    .collect();
                let child_path = crate::pointer::join(path, &index.to_string());
                let placeholder = ResolvedNode {
                    value: ResolvedValue::Null,
                    tag: None,
                    origin: child.origin,
                    opaque: false,
                    number_class: 0,
                };
                let owned = std::mem::replace(child, placeholder);
                *child = self.complete(owned, &item_schemas, &child_path, level.child())?;
            }
        }
        Ok(node)
    }

    // Finish node-local preset work before descending. Keeping these candidate
    // accumulators out of the recursive frame preserves the 128-level contract
    // on the ordinary caller stack, including unoptimized builds.
    fn complete_node_defaults(
        &mut self,
        node: &mut ResolvedNode<'a>,
        mut schemas: Vec<SchemaView<'a>>,
        path: &str,
        level: CompletionLevel,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let depth = level.depth;
        if level.apply_node_defaults && matches!(node.value, ResolvedValue::Mapping(_)) {
            // Native default annotations can introduce a tag whose schema owns
            // additional defaults. Discover every tag assertion before applying
            // payloads, so generated values never become supplied precedence.
            loop {
                schemas = self.applicable(&schemas, node, path, false, true)?;
                if node.tag().is_some() {
                    break;
                }
                if level.revisit
                    && let Some(pending) = &self.pending_defaults
                    && pending.scope(path, node, &mut self.budget)? != stage::DefaultScope::Complete
                {
                    break;
                }
                let Some(tag) = self.discover_default_tag(&schemas, Some(node), path)? else {
                    break;
                };
                node.tag = Some(tag);
            }
            schemas = self.applicable(&schemas, node, path, false, true)?;
            let DefaultLayers {
                tag_owned,
                tag_branches,
                branch_fallbacks,
            } = self.default_layers(node, path)?;
            let supplied_tag = node.tag.as_ref().is_some_and(|tag| tag.supplied);
            let mut baseline = None;
            for schema in &schemas {
                if tag_owned.contains(&schema.id())
                    || tag_branches.contains(&schema.id())
                    || branch_fallbacks.contains(&schema.id())
                    || supplied_tag
                {
                    continue;
                }
                let Some(default) = self.default_value(*schema, path) else {
                    continue;
                };
                if !default.is_object() {
                    continue;
                }
                let Some(candidate) = from_default(
                    default,
                    *schema,
                    depth,
                    &mut self.budget,
                    path,
                    Some(node),
                    self.pending_defaults.as_ref().filter(|_| level.revisit),
                )?
                else {
                    continue;
                };
                if node.tag.as_ref().zip(candidate.tag.as_ref()).is_some_and(
                    |(current, candidate)| !same_tag_identity(self.schemas, current, candidate),
                ) {
                    continue;
                }
                if let Some(prior) = &mut baseline {
                    self.merge_candidates(prior, candidate, Some(node), false, path);
                } else {
                    baseline = Some(candidate);
                }
            }
            if let Some(default) = baseline {
                apply_default(self.schemas, node, default, false);
            }
            for (owned, fallback_ids) in [
                (true, tag_owned),
                (true, tag_branches),
                (false, branch_fallbacks),
            ] {
                let mut fallback = None;
                for schema in &schemas {
                    if !fallback_ids.contains(&schema.id()) {
                        continue;
                    }
                    let Some(default) = self.default_value(*schema, path) else {
                        continue;
                    };
                    if !default.is_object() {
                        continue;
                    }
                    let Some(candidate) = from_default(
                        default,
                        *schema,
                        depth,
                        &mut self.budget,
                        path,
                        Some(node),
                        self.pending_defaults.as_ref().filter(|_| level.revisit),
                    )?
                    else {
                        continue;
                    };
                    if let Some(prior) = &mut fallback {
                        self.merge_candidates(prior, candidate, Some(node), owned, path);
                    } else {
                        fallback = Some(candidate);
                    }
                }
                if let Some(fallback) = fallback {
                    apply_default(self.schemas, node, fallback, owned);
                }
            }
        } else {
            schemas = self.applicable(&schemas, node, path, false, true)?;
        }
        Ok(schemas)
    }

    fn complete_missing(
        &mut self,
        schemas: &[SchemaView<'a>],
        path: &str,
        depth: usize,
        origin: Origin<'a>,
        required: bool,
    ) -> Result<Option<ResolvedNode<'a>>, ResolveError> {
        let Some((candidate, schemas)) =
            self.missing_candidate(schemas, path, depth, origin, required)?
        else {
            return Ok(None);
        };
        self.complete(candidate, &schemas, path, CompletionLevel::merged(depth))
            .map(Some)
    }

    // Candidate/default accumulators must be dropped before descending into a
    // newly synthesized container, just as in complete_node_defaults. Keeping
    // them in a recursive frame exhausts the ordinary stack before depth 128.
    fn missing_candidate(
        &mut self,
        schemas: &[SchemaView<'a>],
        path: &str,
        depth: usize,
        origin: Origin<'a>,
        required: bool,
    ) -> Result<Option<(ResolvedNode<'a>, Vec<SchemaView<'a>>)>, ResolveError> {
        if !required
            && schemas.iter().any(|schema| {
                schema.value().get("x-daml-native-optional") == Some(&JsonValue::Bool(true))
            })
        {
            return Ok(None);
        }
        let mut probe = untagged_placeholder(origin);
        let mut schemas = schemas.to_vec();
        loop {
            schemas = self.applicable(&schemas, &probe, path, required, false)?;
            if probe.tag().is_some() {
                break;
            }
            let Some(tag) = self.discover_default_tag(&schemas, None, path)? else {
                break;
            };
            probe.tag = Some(tag);
        }
        schemas = self.applicable(&schemas, &probe, path, required, false)?;
        let DefaultLayers {
            tag_owned,
            tag_branches,
            branch_fallbacks,
        } = self.default_layers(&probe, path)?;
        let mut completed = None;
        for schema in &schemas {
            if tag_owned.contains(&schema.id())
                || tag_branches.contains(&schema.id())
                || branch_fallbacks.contains(&schema.id())
            {
                continue;
            }
            let Some(default) = self.default_value(*schema, path) else {
                continue;
            };
            let candidate =
                from_default(default, *schema, depth, &mut self.budget, path, None, None)?
                    .expect("a default at an absent destination is retained");
            if probe
                .tag
                .as_ref()
                .zip(candidate.tag.as_ref())
                .is_some_and(|(current, candidate)| {
                    !same_tag_identity(self.schemas, current, candidate)
                })
            {
                continue;
            }
            if let Some(prior) = &mut completed {
                self.merge_candidates(prior, candidate, None, false, path);
            } else {
                completed = Some(candidate);
            }
        }
        for (owned, fallback_ids) in [
            (true, &tag_owned),
            (true, &tag_branches),
            (false, &branch_fallbacks),
        ] {
            let mut fallback = None;
            for schema in &schemas {
                if !fallback_ids.contains(&schema.id()) {
                    continue;
                }
                let Some(default) = self.default_value(*schema, path) else {
                    continue;
                };
                let Some(candidate) = from_default(
                    default,
                    *schema,
                    depth,
                    &mut self.budget,
                    path,
                    completed.as_ref(),
                    None,
                )?
                else {
                    continue;
                };
                if let Some(prior) = &mut fallback {
                    self.merge_candidates(prior, candidate, completed.as_ref(), owned, path);
                } else {
                    fallback = Some(candidate);
                }
            }
            if let Some(fallback) = fallback {
                if let Some(prior) = &mut completed {
                    apply_default(self.schemas, prior, fallback, owned);
                } else {
                    completed = Some(fallback);
                }
            }
        }
        if completed.is_none() {
            if !required {
                return Ok(None);
            }
            let schema = schemas
                .iter()
                .find(|schema| schema.container_kind().is_some());
            let Some(schema) = schema else {
                return Ok(None);
            };
            let value = match schema.container_kind() {
                Some("object") => ResolvedValue::Mapping(BTreeMap::new()),
                Some("array") => ResolvedValue::Sequence(Vec::new()),
                _ => return Ok(None),
            };
            let implicit = ResolvedNode {
                value,
                tag: None,
                origin: Origin::Default(schema.location()),
                opaque: false,
                number_class: 0,
            };
            self.budget.add_node(&implicit, path)?;
            let mut implicit = implicit;
            self.budget.classify(&mut implicit, path)?;
            return Ok(Some((implicit, schemas)));
        }
        Ok(completed.map(|candidate| (candidate, schemas)))
    }

    fn conjunction(
        &mut self,
        schemas: &[SchemaView<'a>],
        node: &ResolvedNode<'a>,
        path: &str,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let tag = if let Some(tag) = node.tag() {
            Some(tag_schema(self.schemas, node).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownTag,
                    node,
                    format!("no schema is bound to tag {tag:?}"),
                    None,
                    path,
                )
            })?)
        } else {
            None
        };
        self.schema_conjunction(schemas.iter().copied().chain(tag), node, path)
    }

    // Expand schema structure without assigning the instance tag to a branch.
    fn schema_conjunction(
        &mut self,
        schemas: impl IntoIterator<Item = SchemaView<'a>>,
        node: &ResolvedNode<'a>,
        path: &str,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = 0;
        for schema in schemas {
            self.budget.step(node, path)?;
            if seen.insert(schema.id()) {
                out.push(schema);
            }
        }
        while cursor < out.len() {
            let view = out[cursor];
            cursor += 1;
            self.budget.step(node, path)?;
            if let Some(reference) = view.referenced()
                && seen.insert(reference.id())
            {
                out.push(reference);
            }
            for child in view.array_children("allOf") {
                if seen.insert(child.id()) {
                    out.push(child);
                }
            }
        }
        Ok(out)
    }

    fn applicable(
        &mut self,
        schemas: &[SchemaView<'a>],
        node: &ResolvedNode<'a>,
        path: &str,
        missing: bool,
        present: bool,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut schemas = schemas.to_vec();
        let mut skipped = Vec::new();
        loop {
            schemas = self.conjunction(&schemas, node, path)?;
            let Some(union) = self.next_union(&schemas, &skipped, node, path, missing, present)?
            else {
                return Ok(schemas);
            };
            self.budget.step(node, path)?;
            let branch = self.search.branch(path, &union);
            skipped.push((union.schema.id(), union.keyword));
            schemas.push(branch);
        }
    }

    fn next_union(
        &mut self,
        schemas: &[SchemaView<'a>],
        skipped: &[(SchemaId, &'static str)],
        node: &ResolvedNode<'a>,
        path: &str,
        missing: bool,
        present: bool,
    ) -> Result<Option<Union<'a>>, ResolveError> {
        self.budget.charge_work(schemas.len(), node, path)?;
        if !schemas.iter().any(|schema| {
            schema.array_children("anyOf").len() != 0 || schema.array_children("oneOf").len() != 0
        }) {
            return Ok(None);
        }
        // Stages may replace values and generated assertions. Only retained
        // contracts can prove an alternative impossible across such edits.
        let context: Cow<'_, [SchemaView<'a>]> = if self.budget.stage.is_some() {
            let asserted = node
                .tag
                .as_ref()
                .filter(|tag| tag.supplied)
                .and_then(|_| tag_schema(self.schemas, node));
            let retained = self.contracts.get(path).into_iter().flatten().copied();
            let selected = self
                .search
                .branches
                .get(path)
                .into_iter()
                .flat_map(|branches| branches.views.values().copied());
            let retained = asserted
                .into_iter()
                .chain(retained)
                .chain(selected)
                .collect::<Vec<_>>();
            Cow::Owned(self.schema_conjunction(retained, node, path)?)
        } else {
            Cow::Borrowed(schemas)
        };
        for schema in schemas {
            for keyword in ["anyOf", "oneOf"] {
                if skipped.contains(&(schema.id(), keyword)) {
                    continue;
                }
                if path.is_empty()
                    && keyword == "anyOf"
                    && let Some(fixed) = self.search.fixed_root_any_of.get(&schema.id())
                {
                    return Ok(Some(Union {
                        schema: *schema,
                        keyword,
                        branches: vec![*fixed],
                    }));
                }
                // Validation-only unions need no completion proof or search.
                // In particular, do not intersect large absent-field domains
                // before a common default has supplied their actual value.
                let mut needs_selection = false;
                for branch in schema.array_children(keyword) {
                    self.budget.step(node, path)?;
                    if branch.has_defaults()
                        || (missing
                            && self.schema_container_shapes(
                                branch,
                                node,
                                path,
                                &mut HashSet::new(),
                            )? != 0)
                        || self.has_implicit_branch_work(schemas, &[branch], node, path, 0)?
                    {
                        needs_selection = true;
                        break;
                    }
                }
                if !needs_selection {
                    continue;
                }
                let mut branches = Vec::new();
                let mut completion_work = false;
                for branch in schema.array_children(keyword) {
                    self.budget.step(node, path)?;
                    if (present || missing)
                        && !self.compatible_branch(
                            &context,
                            branch,
                            node,
                            path,
                            present && self.budget.stage.is_none(),
                            &mut HashSet::new(),
                        )?
                    {
                        continue;
                    }
                    branches.push(branch);
                    if !completion_work {
                        completion_work = branch.has_defaults()
                            || (missing
                                && self.schema_container_shapes(
                                    branch,
                                    node,
                                    path,
                                    &mut HashSet::new(),
                                )? != 0)
                            || self.has_implicit_branch_work(schemas, &[branch], node, path, 0)?;
                    }
                }
                if completion_work {
                    return Ok(Some(Union {
                        schema: *schema,
                        keyword,
                        branches,
                    }));
                }
            }
        }
        Ok(None)
    }

    // This is an impossibility proof, not a second validator. Ignore unknown
    // constraints and overlapping oneOf arms; the engine still checks every
    // complete candidate against the original, unpruned union.
    fn compatible_branch(
        &mut self,
        common: &[SchemaView<'a>],
        branch: SchemaView<'a>,
        node: &ResolvedNode<'a>,
        path: &str,
        present: bool,
        active: &mut HashSet<SchemaId>,
    ) -> Result<bool, ResolveError> {
        if active.len() >= crate::schema::profile::MAX_SCHEMA_DEPTH || !active.insert(branch.id()) {
            return Ok(true);
        }
        let branch_schemas = self.schema_conjunction([branch], node, path)?;
        let combined =
            self.schema_conjunction(common.iter().chain(&branch_schemas).copied(), node, path)?;
        let result = (|| {
            if combined
                .iter()
                .any(|schema| schema.value() == &JsonValue::Bool(false))
                || !self.finite_domains_overlap(&combined, present.then_some(node), node, path)?
            {
                return Ok(false);
            }
            let object = combined
                .iter()
                .any(|schema| schema.container_kind() == Some("object"))
                || (self.budget.stage.is_none() && matches!(node.value, ResolvedValue::Mapping(_)));
            if object {
                let supplied_lookup_work = if present {
                    self.supplied_property_lookup_work(node, path)?
                } else {
                    0
                };
                let mut names = self.property_names(&combined, node, path)?;
                while let Some(key) = names.next(&mut self.budget, node, path)? {
                    self.budget.charge_work(supplied_lookup_work, node, path)?;
                    let supplied = present.then(|| node.get(key)).flatten();
                    if supplied.is_none() && !self.requires_property(&combined, key, node, path)? {
                        continue;
                    }
                    let properties = self.property_schemas(&combined, key, node, path)?;
                    let properties = self.schema_conjunction(properties, node, path)?;
                    if !self.finite_domains_overlap(&properties, supplied, node, path)? {
                        return Ok(false);
                    }
                }
            }
            // Only inspect branch-owned unions. Re-entering the common unions
            // would mistake their own alternatives for conjunctive constraints.
            for schema in &branch_schemas {
                for keyword in ["anyOf", "oneOf"] {
                    let mut alternatives = schema.array_children(keyword);
                    let Some(first) = alternatives.next() else {
                        continue;
                    };
                    let mut possible = false;
                    for alternative in std::iter::once(first).chain(alternatives) {
                        if self.compatible_branch(
                            &combined,
                            alternative,
                            node,
                            path,
                            present,
                            active,
                        )? {
                            possible = true;
                            break;
                        }
                    }
                    if !possible {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        })();
        active.remove(&branch.id());
        result
    }

    fn finite_domains_overlap(
        &mut self,
        schemas: &[SchemaView<'a>],
        supplied: Option<&ResolvedNode<'_>>,
        node: &ResolvedNode<'a>,
        path: &str,
    ) -> Result<bool, ResolveError> {
        self.budget.charge_work(schemas.len(), node, path)?;
        if let Some(supplied) = supplied.filter(|supplied| {
            !matches!(
                supplied.value,
                ResolvedValue::Mapping(_) | ResolvedValue::Sequence(_)
            )
        }) {
            let input_work = 1usize.saturating_add(match &supplied.value {
                ResolvedValue::String(value) => value.len().div_ceil(64),
                ResolvedValue::Temporal(value) => value.as_str().len().div_ceil(64),
                ResolvedValue::Number(value) => value.as_str().len().div_ceil(64),
                _ => 0,
            });
            for schema in schemas {
                self.budget.charge_work(
                    schema.finite_value_work().saturating_add(input_work),
                    node,
                    path,
                )?;
                if !schema.permits_finite_node(supplied) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let Some((domain, values)) = schemas
            .iter()
            .filter_map(|schema| schema.finite_values().map(|values| (schema, values)))
            .min_by_key(|(_, values)| values.len())
        else {
            return Ok(true);
        };
        for value in values {
            // The local compiled body includes the entire const/enum payload.
            // Charge both sides before potentially deep JSON equality checks.
            self.budget
                .charge_work(domain.finite_value_work(), node, path)?;
            let mut possible = true;
            for schema in schemas {
                self.budget
                    .charge_work(schema.finite_value_work(), node, path)?;
                if !schema.permits_finite_value(value) {
                    possible = false;
                    break;
                }
            }
            if possible {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Bit 0 is an object shape; bit 1 is an array shape. Discovery considers
    // possible construction only; the ordinary validator still enforces every
    // selected constraint and rejects incompatible combinations.
    fn schema_container_shapes(
        &mut self,
        schema: SchemaView<'a>,
        node: &ResolvedNode<'a>,
        path: &str,
        seen: &mut HashSet<SchemaId>,
    ) -> Result<u8, ResolveError> {
        self.budget.step(node, path)?;
        if !seen.insert(schema.id()) {
            return Ok(0);
        }
        let mut shapes = match schema.container_kind() {
            Some("object") => 1,
            Some("array") => 2,
            _ => 0,
        };
        if let Some(reference) = schema.referenced() {
            shapes |= self.schema_container_shapes(reference, node, path, seen)?;
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            for child in schema.array_children(keyword) {
                shapes |= self.schema_container_shapes(child, node, path, seen)?;
            }
        }
        Ok(shapes)
    }

    fn required_container_shapes(
        &mut self,
        schemas: &[SchemaView<'a>],
        key: &str,
        node: &ResolvedNode<'a>,
        path: &str,
    ) -> Result<u8, ResolveError> {
        if !self.requires_property(schemas, key, node, path)? {
            return Ok(0);
        }
        let mut shapes = 0;
        let mut seen = HashSet::new();
        for schema in schemas {
            if let Some(property) = self.property_schema(*schema, key, node, path)? {
                shapes |= self.schema_container_shapes(property, node, path, &mut seen)?;
            }
        }
        Ok(shapes)
    }

    // Ask what this branch adds, not whether the common context already has
    // work. Otherwise unrelated validation-only unions form a Cartesian search.
    fn has_implicit_branch_work(
        &mut self,
        common: &[SchemaView<'a>],
        branch: &[SchemaView<'a>],
        node: &ResolvedNode<'a>,
        path: &str,
        discovery_depth: usize,
    ) -> Result<bool, ResolveError> {
        self.budget.step(node, path)?;
        if discovery_depth >= crate::schema::profile::MAX_SCHEMA_DEPTH {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "implicit container discovery exceeds depth bound",
                None,
                path,
            ));
        }
        let common = self.conjunction(common, node, path)?;
        let branch = self.schema_conjunction(branch.iter().copied(), node, path)?;
        if branch.is_empty() {
            return Ok(false);
        }
        let mut combined = common.clone();
        combined.extend_from_slice(&branch);
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                let mut names = self.property_names(&combined, node, path)?;
                let supplied_lookup_work = self.supplied_property_lookup_work(node, path)?;
                while let Some(key) = names.next(&mut self.budget, node, path)? {
                    self.budget.charge_work(supplied_lookup_work, node, path)?;
                    if fields.contains_key(key) {
                        continue;
                    }
                    let shapes = self.required_container_shapes(&combined, key, node, path)?;
                    if shapes == 0 {
                        continue;
                    }
                    let common_shapes = self.required_container_shapes(&common, key, node, path)?;
                    if common_shapes == 0 {
                        return Ok(true);
                    }
                    // Common constraints already create this container. A branch
                    // can still add completion work inside its initially empty value.
                    let branch_children = self.property_schemas(&branch, key, node, path)?;
                    if branch_children.is_empty() {
                        continue;
                    }
                    let common_children = self.property_schemas(&common, key, node, path)?;
                    let child_path = crate::pointer::join(path, key);
                    for (shape, value) in [
                        (1, ResolvedValue::Mapping(BTreeMap::new())),
                        (2, ResolvedValue::Sequence(Vec::new())),
                    ] {
                        if common_shapes & shape == 0 {
                            continue;
                        }
                        let probe = ResolvedNode {
                            value,
                            tag: None,
                            origin: node.origin,
                            opaque: false,
                            number_class: 0,
                        };
                        if self.has_implicit_branch_work(
                            &common_children,
                            &branch_children,
                            &probe,
                            &child_path,
                            discovery_depth + 1,
                        )? {
                            return Ok(true);
                        }
                    }
                }
                for (key, value) in fields {
                    self.budget.step(value, path)?;
                    let branch_children = self.property_schemas(&branch, key, value, path)?;
                    if branch_children.is_empty() {
                        continue;
                    }
                    let common_children = self.property_schemas(&common, key, value, path)?;
                    if self.has_implicit_branch_work(
                        &common_children,
                        &branch_children,
                        value,
                        &crate::pointer::join(path, key),
                        discovery_depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
            }
            ResolvedValue::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.budget.step(item, path)?;
                    self.budget.charge_work(branch.len(), item, path)?;
                    let branch_children: Vec<_> = branch
                        .iter()
                        .filter_map(|schema| schema.item(index))
                        .collect();
                    if branch_children.is_empty() {
                        continue;
                    }
                    self.budget.charge_work(common.len(), item, path)?;
                    let common_children: Vec<_> = common
                        .iter()
                        .filter_map(|schema| schema.item(index))
                        .collect();
                    if self.has_implicit_branch_work(
                        &common_children,
                        &branch_children,
                        item,
                        &crate::pointer::join(path, &index.to_string()),
                        discovery_depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
        // Only branch-owned unions are explored here. Common unions are handled
        // by their own selection; re-entering them would create discovery cycles.
        for schema in &branch {
            for keyword in ["anyOf", "oneOf"] {
                for nested in schema.array_children(keyword) {
                    if self.has_implicit_branch_work(
                        &combined,
                        &[nested],
                        node,
                        path,
                        discovery_depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn property_schema(
        &mut self,
        schema: SchemaView<'a>,
        key: &str,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<Option<SchemaView<'a>>, ResolveError> {
        self.budget
            .charge_work(schema.property_lookup_work(), node, path)?;
        Ok(schema.property(key))
    }

    fn property_schemas(
        &mut self,
        schemas: &[SchemaView<'a>],
        key: &str,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut properties = Vec::new();
        for schema in schemas {
            if let Some(property) = self.property_schema(*schema, key, node, path)? {
                properties.push(property);
            }
        }
        Ok(properties)
    }

    fn requires_property(
        &mut self,
        schemas: &[SchemaView<'a>],
        key: &str,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<bool, ResolveError> {
        for schema in schemas {
            self.budget
                .charge_work(schema.required_lookup_work(), node, path)?;
            if schema.requires_property(key) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn supplied_property_lookup_work(
        &mut self,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<usize, ResolveError> {
        let mut lookup_work = 1usize;
        if let ResolvedValue::Mapping(fields) = &node.value {
            for name in fields.keys() {
                let work = name.len().div_ceil(64).saturating_add(1);
                self.budget.charge_work(work, node, path)?;
                lookup_work = lookup_work.saturating_add(work);
            }
        }
        Ok(lookup_work)
    }

    fn property_names<'s>(
        &mut self,
        schemas: &'s [SchemaView<'a>],
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<PropertyNames<'a, impl Iterator<Item = &'a str> + use<'a, 's>>, ResolveError> {
        self.budget.charge_work(schemas.len(), node, path)?;
        let capacity = schemas.iter().fold(0usize, |count, schema| {
            count.saturating_add(schema.property_count())
        });
        // Reserve before allocation. Capacity covers every yielded name, so
        // insertion never grows the table and rehashes previously charged text.
        self.budget.charge_work(capacity, node, path)?;
        Ok(PropertyNames {
            names: schemas.iter().flat_map(|schema| schema.property_names()),
            seen: HashSet::with_capacity(capacity),
        })
    }

    fn known_tags(&mut self, node: &ResolvedNode<'_>, path: &str) -> Result<(), ResolveError> {
        self.budget.step(node, path)?;
        if node.tag.is_some() && tag_schema(self.schemas, node).is_none() {
            return Err(failure(
                ResolveErrorKind::UnknownTag,
                node,
                format!("no schema is bound to tag {:?}", node.tag().unwrap()),
                None,
                path,
            ));
        }
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.known_tags(child, &crate::pointer::join(path, key))?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (index, child) in items.iter().enumerate() {
                    self.known_tags(child, &crate::pointer::join(path, &index.to_string()))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn admit_tags(
        &mut self,
        node: &mut ResolvedNode<'a>,
        path: &str,
        depth: usize,
        admission: TagAdmission,
    ) -> Result<(), ResolveError> {
        self.budget.step(node, path)?;
        if depth > 128 {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "resolved data exceeds depth 128",
                None,
                path,
            ));
        }
        // PinCandidate visits the entire snapshot, including opaque descendants
        // that completion deliberately skips, before the host can edit it again.
        if matches!(admission, TagAdmission::PinCandidate)
            && let Some(branches) = self.search.branches.get_mut(path)
        {
            branches.materialized = true;
        }
        if let Some(tag) = node.tag() {
            let schema = tag_schema(self.schemas, node).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownTag,
                    node,
                    format!("no schema is bound to tag {tag:?}"),
                    None,
                    path,
                )
            })?;
            if !matches!(admission, TagAdmission::PinCandidate) {
                Self::check_node(node, &schema, path, &mut self.budget)?;
            }
            let tag = node.tag.as_ref().expect("checked asserted tag is present");
            if !matches!(admission, TagAdmission::Validate)
                && (tag.binding.is_none() || !matches!(tag.origin, Origin::Semantic))
            {
                let canonical = self
                    .schemas
                    .canonical_binding_schema(tag.binding.as_deref().unwrap_or(tag.name.as_ref()))
                    .expect("checked tag has a canonical registry contract");
                let binding = canonical.binding();
                if binding.len() > crate::semantic::MAX_SCALAR_BYTES {
                    return Err(failure(
                        ResolveErrorKind::LimitExceeded,
                        node,
                        "canonical tag binding exceeds 1 MiB byte bound",
                        None,
                        path,
                    ));
                }
                let previous_bytes = tag.binding.as_ref().map_or(0, |binding| binding.len());
                self.budget.add_byte_count(
                    node,
                    binding.len().saturating_sub(previous_bytes),
                    path,
                )?;
                node.tag
                    .as_mut()
                    .expect("checked asserted tag is present")
                    .binding = Some(binding);
            }
        }
        match &mut node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.admit_tags(
                        child,
                        &crate::pointer::join(path, key),
                        depth + 1,
                        admission,
                    )?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (i, child) in items.iter_mut().enumerate() {
                    self.admit_tags(
                        child,
                        &crate::pointer::join(path, &i.to_string()),
                        depth + 1,
                        admission,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn check_node(
        node: &ResolvedNode<'_>,
        schema: &SchemaView<'a>,
        path: &str,
        budget: &mut Budget<'_>,
    ) -> Result<(), ResolveError> {
        // A successful validity probe needs no diagnostic trees. Reserve both
        // operations independently: failures still pay for the engine's eager
        // error collection before it runs, using the same cumulative budget.
        budget.charge_work(
            schema.validity_cost(node, budget.limits.max_steps.saturating_sub(budget.steps)),
            node,
            path,
        )?;
        if schema.is_valid(node) {
            return Ok(());
        }
        budget.charge_work(
            schema.evaluation_cost(node, budget.limits.max_steps.saturating_sub(budget.steps)),
            node,
            path,
        )?;
        schema.validate(node).map_err(|failure_info| {
            let full_path = format!("{path}{}", failure_info.path);
            let failing = node.at_pointer(&failure_info.path).unwrap_or(node);
            let kind = ResolveErrorKind::SchemaViolation;
            failure(
                kind,
                failing,
                failure_info.message,
                Some(failure_info.schema),
                &full_path,
            )
        })
    }
    fn default_value(&self, schema: SchemaView<'a>, path: &str) -> Option<&'a JsonValue> {
        if path.is_empty() && self.search.suppressed_root_default == Some(schema.id()) {
            None
        } else {
            schema.value().get("default")
        }
    }

    fn discover_default_tag(
        &mut self,
        schemas: &[SchemaView<'a>],
        supplied: Option<&ResolvedNode<'a>>,
        path: &str,
    ) -> Result<Option<ResolvedTag<'a>>, ResolveError> {
        let mut discovered: Option<ResolvedTag<'a>> = None;
        for schema in schemas {
            let Some(default) = self.default_value(*schema, path) else {
                continue;
            };
            // Existing data only merges object defaults. A tag on a skipped
            // scalar/sequence default must not leak onto a supplied mapping.
            if supplied.is_some() && !default.is_object() {
                continue;
            }
            if supplied.is_some() && self.budget.stage.is_some() {
                let probe = untagged_placeholder(Origin::Default(schema.location()));
                let tag = default_tag(default, *schema, &mut self.budget, path)?;
                if self
                    .budget
                    .defer_default(default, *schema, tag.as_ref(), &probe, path)?
                {
                    continue;
                }
            }
            let Some(tag) = default_tag(default, *schema, &mut self.budget, path)? else {
                continue;
            };
            if let Some(prior) = &mut discovered {
                if !same_tag_identity(self.schemas, prior, &tag)
                    && self.search.conflicting_default_tag(&tag, path) == 1
                {
                    *prior = tag;
                }
            } else {
                discovered = Some(tag);
            }
        }
        Ok(discovered)
    }

    fn merge_candidates(
        &mut self,
        target: &mut ResolvedNode<'a>,
        mut incoming: ResolvedNode<'a>,
        supplied: Option<&ResolvedNode<'a>>,
        tag_owned: bool,
        path: &str,
    ) {
        if supplied.is_some_and(|node| node.opaque) {
            return;
        }
        if let Some(supplied) = supplied {
            if !tag_owned && supplied.tag.as_ref().is_some_and(|tag| tag.supplied) {
                return;
            }
            if !matches!(supplied.value, ResolvedValue::Mapping(_)) {
                return;
            }
            if incoming.opaque {
                return;
            }
            if target.opaque && matches!(incoming.value, ResolvedValue::Mapping(_)) {
                *target = incoming;
                return;
            }
            if !tag_owned && let Some(tag) = supplied.tag.as_ref() {
                if incoming
                    .tag
                    .as_ref()
                    .is_some_and(|incoming| !same_tag_identity(self.schemas, incoming, tag))
                {
                    return;
                }
                if target
                    .tag
                    .as_ref()
                    .is_some_and(|current| !same_tag_identity(self.schemas, current, tag))
                {
                    *target = incoming;
                    return;
                }
            }
            match (&target.value, &incoming.value) {
                (ResolvedValue::Mapping(_), ResolvedValue::Mapping(_)) => {}
                (_, ResolvedValue::Mapping(_)) => {
                    *target = incoming;
                    return;
                }
                _ => return,
            }
        }
        let both_maps = !target.opaque
            && !incoming.opaque
            && matches!(
                (&target.value, &incoming.value),
                (ResolvedValue::Mapping(_), ResolvedValue::Mapping(_))
            );
        if !both_maps {
            if !self.same_values(target, &incoming)
                && self.search.conflicting_default(&incoming, path) == 1
            {
                *target = incoming;
            }
            return;
        }
        if (tag_owned || supplied.and_then(ResolvedNode::tag).is_none())
            && target
                .tag
                .as_ref()
                .zip(incoming.tag.as_ref())
                .is_some_and(|(target, incoming)| {
                    !same_tag_identity(self.schemas, target, incoming)
                })
        {
            if self.search.conflicting_default(&incoming, path) == 1 {
                target.tag = incoming.tag.take();
            }
        } else if target.tag.is_none() {
            target.tag = incoming.tag.take();
        }
        if let (ResolvedValue::Mapping(target), ResolvedValue::Mapping(incoming)) =
            (&mut target.value, incoming.value)
        {
            for (key, value) in incoming {
                if let Some(existing) = target.get_mut(key.as_ref()) {
                    self.merge_candidates(
                        existing,
                        value,
                        supplied.and_then(|node| node.get(&key)),
                        false,
                        &crate::pointer::join(path, &key),
                    );
                } else {
                    target.insert(key, value);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CompletionLevel {
    depth: usize,
    apply_node_defaults: bool,
    // Newly materialized subtrees need their first completion even when a
    // continuation suppresses presets on previously completed records.
    revisit: bool,
}

impl CompletionLevel {
    const fn existing(depth: usize) -> Self {
        Self {
            depth,
            apply_node_defaults: true,
            revisit: true,
        }
    }

    const fn merged(depth: usize) -> Self {
        Self {
            depth,
            apply_node_defaults: false,
            revisit: false,
        }
    }

    const fn child(self) -> Self {
        Self {
            depth: self.depth + 1,
            apply_node_defaults: true,
            revisit: self.revisit,
        }
    }
}

struct Union<'a> {
    schema: SchemaView<'a>,
    keyword: &'static str,
    branches: Vec<SchemaView<'a>>,
}

#[derive(Default)]
struct SelectedBranches<'a> {
    views: HashMap<(SchemaId, &'static str), SchemaView<'a>>,
    // Probing an absent optional value must not make it required.
    materialized: bool,
}

/// A depth-first, mixed-radix search through branch/default choices. A changed
/// choice truncates its suffix, so every replay reaches the same choice prefix
/// even when the next branch has a different shape. All replays share Budget.
#[derive(Default)]
struct CompletionSearch<'a> {
    choices: Vec<usize>,
    arities: Vec<usize>,
    branches: HashMap<String, SelectedBranches<'a>>,
    fixed_root_any_of: HashMap<SchemaId, SchemaView<'a>>,
    suppressed_root_default: Option<SchemaId>,
    conflict: Option<ResolveError>,
}

impl<'a> CompletionSearch<'a> {
    fn start(&mut self) {
        self.arities.clear();
        self.branches.clear();
        self.conflict = None;
    }

    fn choose(&mut self, arity: usize) -> usize {
        let cursor = self.arities.len();
        self.arities.push(arity);
        if cursor == self.choices.len() {
            self.choices.push(0);
        }
        self.choices[cursor]
    }

    fn branch(&mut self, path: &str, union: &Union<'a>) -> SchemaView<'a> {
        let id = union.schema.id();
        let keyword = union.keyword;
        if let Some(view) = self
            .branches
            .get(path)
            .and_then(|branches| branches.views.get(&(id, keyword)))
        {
            return *view;
        }
        let fixed = if path.is_empty() && keyword == "anyOf" {
            self.fixed_root_any_of.get(&id).copied()
        } else {
            None
        };
        let branch = if let Some(branch) = fixed {
            branch
        } else {
            let selected = if union.branches.len() == 1 {
                0
            } else {
                self.choose(union.branches.len())
            };
            union.branches[selected]
        };
        let branches = self.branches.entry(path.to_owned()).or_default();
        branches.views.insert((id, keyword), branch);
        branch
    }

    fn advance(&mut self) -> bool {
        self.choices.truncate(self.arities.len());
        for index in (0..self.arities.len()).rev() {
            if self.choices[index] + 1 < self.arities[index] {
                self.choices[index] += 1;
                self.choices.truncate(index + 1);
                return true;
            }
        }
        false
    }

    fn conflicting_default(&mut self, node: &ResolvedNode<'_>, path: &str) -> usize {
        if self.conflict.is_none() {
            let Origin::Default(location) = node.origin else {
                unreachable!("only declared defaults are merged")
            };
            self.conflict = Some(default_failure(
                location,
                path,
                "applicable schemas provide conflicting defaults",
            ));
        }
        self.choose(2)
    }

    fn conflicting_default_tag(&mut self, tag: &ResolvedTag<'_>, path: &str) -> usize {
        if self.conflict.is_none() {
            let Origin::Default(location) = tag.origin else {
                unreachable!("only default tags are discovered")
            };
            self.conflict = Some(default_failure(
                location,
                path,
                "applicable schemas provide conflicting default tags",
            ));
        }
        self.choose(2)
    }
}

struct Budget<'s> {
    limits: ResolutionLimits,
    nodes: usize,
    bytes: usize,
    steps: usize,
    stage: Option<&'s mut dyn stage::StageHooks>,
}
impl Budget<'_> {
    fn step(&mut self, node: &ResolvedNode<'_>, path: &str) -> Result<(), ResolveError> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > self.limits.max_steps {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "resolution step limit exceeded",
                None,
                path,
            ));
        }
        Ok(())
    }
    fn charge_work(
        &mut self,
        work: usize,
        node: &ResolvedNode<'_>,
        path: &str,
    ) -> Result<(), ResolveError> {
        self.steps = self.steps.saturating_add(work);
        if self.steps > self.limits.max_steps {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "schema validation work exceeds step limit",
                None,
                path,
            ));
        }
        Ok(())
    }
    fn add_node(&mut self, node: &ResolvedNode<'_>, path: &str) -> Result<(), ResolveError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "resolved node limit exceeded",
                None,
                path,
            ));
        }
        Ok(())
    }
    fn add_bytes(
        &mut self,
        node: &ResolvedNode<'_>,
        text: &str,
        path: &str,
    ) -> Result<(), ResolveError> {
        self.add_byte_count(node, text.len(), path)
    }

    fn add_byte_count(
        &mut self,
        node: &ResolvedNode<'_>,
        bytes: usize,
        path: &str,
    ) -> Result<(), ResolveError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limits.max_bytes {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "resolved byte limit exceeded",
                None,
                path,
            ));
        }
        Ok(())
    }

    fn add_semantic_scalar(
        &mut self,
        node: &ResolvedNode<'_>,
        text: &str,
        path: &str,
    ) -> Result<(), ResolveError> {
        if text.len() > crate::semantic::MAX_SCALAR_BYTES {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "semantic scalar exceeds 1 MiB byte bound",
                None,
                path,
            ));
        }
        self.add_bytes(node, text, path)
    }

    fn semantic_children(
        &self,
        node: &ResolvedNode<'_>,
        count: usize,
        path: &str,
    ) -> Result<(), ResolveError> {
        if count > self.limits.max_nodes.saturating_sub(self.nodes) {
            return Err(failure(
                ResolveErrorKind::LimitExceeded,
                node,
                "semantic children exceed remaining node budget",
                None,
                path,
            ));
        }
        Ok(())
    }
}

fn from_semantic<'a>(
    node: &'a SemanticNode,
    depth: usize,
    path: &str,
    budget: &mut Budget<'_>,
) -> Result<ResolvedNode<'a>, ResolveError> {
    from_semantic_with_keys(node, depth, path, budget, true)
}

fn from_semantic_with_keys<'a>(
    node: &'a SemanticNode,
    depth: usize,
    path: &str,
    budget: &mut Budget<'_>,
    require_source_keys: bool,
) -> Result<ResolvedNode<'a>, ResolveError> {
    let mut result = untagged_placeholder(Origin::Semantic);
    budget.add_node(&result, path)?;
    if depth >= 128
        && matches!(
            &node.value,
            SemanticValue::Sequence(_) | SemanticValue::Mapping(_)
        )
    {
        return Err(failure(
            ResolveErrorKind::LimitExceeded,
            &result,
            "semantic collection nesting exceeds 128",
            None,
            path,
        ));
    }
    if let Some(tag) = &node.tag {
        budget.add_semantic_scalar(&result, &tag.name, path)?;
        if let Some(binding) = &tag.binding {
            budget.add_semantic_scalar(&result, binding, path)?;
        }
        if !crate::names::valid_tag(&tag.name)
            || crate::TemporalKind::from_constructor(&tag.name).is_some()
        {
            return Err(failure(
                ResolveErrorKind::InvalidTag,
                &result,
                "semantic tag violates the DAML tag grammar",
                None,
                path,
            ));
        }
        result.tag = Some(ResolvedTag {
            name: Cow::Borrowed(&tag.name),
            binding: tag.binding.as_deref().map(Cow::Borrowed),
            origin: Origin::Semantic,
            supplied: true,
        });
    }
    result.value = match &node.value {
        SemanticValue::Null => ResolvedValue::Null,
        SemanticValue::Bool(value) => ResolvedValue::Bool(*value),
        SemanticValue::Number(number) => {
            let raw = number.as_str();
            budget.add_semantic_scalar(&result, raw, path)?;
            check_number_limits(raw).map_err(|message| {
                failure(ResolveErrorKind::NumericLimit, &result, message, None, path)
            })?;
            ResolvedValue::Number(Cow::Borrowed(number))
        }
        SemanticValue::String(text) => {
            budget.add_semantic_scalar(&result, text, path)?;
            ResolvedValue::String(Cow::Borrowed(text))
        }
        SemanticValue::Temporal(value) => {
            budget.add_semantic_scalar(&result, value.as_str(), path)?;
            ResolvedValue::Temporal(value.as_borrowed())
        }
        SemanticValue::Sequence(items) => {
            budget.semantic_children(&result, items.len(), path)?;
            ResolvedValue::Sequence(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        from_semantic_with_keys(
                            child,
                            depth + 1,
                            &crate::pointer::join(path, &index.to_string()),
                            budget,
                            require_source_keys,
                        )
                    })
                    .collect::<Result<_, _>>()?,
            )
        }
        SemanticValue::Mapping(fields) => {
            budget.semantic_children(&result, fields.len(), path)?;
            let mut resolved = BTreeMap::new();
            for (key, child) in fields {
                budget.add_semantic_scalar(&result, key, path)?;
                let child_path = crate::pointer::join(path, key);
                if require_source_keys && !crate::names::valid_key(key) {
                    return Err(failure(
                        ResolveErrorKind::InvalidKey,
                        &result,
                        "semantic mapping key violates the DAML key grammar",
                        None,
                        &child_path,
                    ));
                }
                let child = from_semantic_with_keys(
                    child,
                    depth + 1,
                    &child_path,
                    budget,
                    require_source_keys,
                )?;
                resolved.insert(Cow::Borrowed(key.as_str()), child);
            }
            ResolvedValue::Mapping(resolved)
        }
    };
    budget.classify(&mut result, path)?;
    Ok(result)
}

fn from_input<'a>(
    document: &'a Document,
    node: &'a Node,
    depth: usize,
    path: &str,
    budget: &mut Budget<'_>,
) -> Result<ResolvedNode<'a>, ResolveError> {
    let origin = Origin::Input(node.span);
    let mut result = ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin,
        opaque: false,
        number_class: 0,
    };
    budget.add_node(&result, path)?;
    if depth > 128 {
        return Err(failure(
            ResolveErrorKind::LimitExceeded,
            &result,
            "input exceeds depth 128",
            None,
            path,
        ));
    }
    if let Some(tag) = document.tag(node) {
        budget.add_bytes(&result, tag, path)?;
        result.tag = Some(ResolvedTag {
            name: Cow::Borrowed(tag),
            binding: None,
            origin: Origin::Input(node.tag.expect("parser tag name has its span")),
            supplied: true,
        });
    }
    result.value = match &node.value {
        Value::Null => ResolvedValue::Null,
        Value::Bool(value) => ResolvedValue::Bool(*value),
        Value::Number(span) => {
            let raw = document.slice(*span).ok_or_else(|| {
                failure(
                    ResolveErrorKind::NumericLimit,
                    &result,
                    "invalid numeric source span",
                    None,
                    path,
                )
            })?;
            budget.add_bytes(&result, raw, path)?;
            ResolvedValue::Number(Cow::Owned(checked_number(raw).map_err(|error| {
                failure(ResolveErrorKind::NumericLimit, &result, error, None, path)
            })?))
        }
        Value::String(_) => {
            let value = document
                .string(node)
                .expect("parser-produced string spans resolve in their document");
            budget.add_bytes(&result, value, path)?;
            ResolvedValue::String(Cow::Borrowed(value))
        }
        Value::Temporal(_) => {
            let value = document.temporal(node).expect("admitted temporal source");
            budget.add_bytes(&result, value.as_str(), path)?;
            ResolvedValue::Temporal(value)
        }
        Value::Sequence(items) => ResolvedValue::Sequence(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    from_input(
                        document,
                        item,
                        depth + 1,
                        &crate::pointer::join(path, &index.to_string()),
                        budget,
                    )
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::Mapping(entries) => {
            let mut fields = BTreeMap::new();
            for entry in entries {
                let key = document
                    .slice(entry.key)
                    .expect("parser-produced keys have valid source spans");
                let child_path = crate::pointer::join(path, key);
                budget.add_bytes(&result, key, &child_path)?;
                let value = from_input(document, &entry.value, depth + 1, &child_path, budget)?;
                fields.insert(Cow::Borrowed(key), value);
            }
            ResolvedValue::Mapping(fields)
        }
    };
    budget.classify(&mut result, path)?;
    Ok(result)
}

fn from_default<'a>(
    value: &'a JsonValue,
    schema: SchemaView<'a>,
    depth: usize,
    budget: &mut Budget<'_>,
    path: &str,
    supplied: Option<&ResolvedNode<'_>>,
    scope: Option<&stage::PendingDefaults>,
) -> Result<Option<ResolvedNode<'a>>, ResolveError> {
    let location = schema.default_origin().unwrap_or(schema.location());
    let origin = Origin::Default(location);
    let mut node = ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin,
        opaque: false,
        number_class: 0,
    };
    let scope_kind = match scope {
        Some(scope) => scope.scope(path, &node, budget)?,
        None => stage::DefaultScope::Complete,
    };
    if scope_kind == stage::DefaultScope::Skip
        || (scope_kind == stage::DefaultScope::Project && !value.is_object())
    {
        return Ok(None);
    }
    node.tag = default_tag(value, schema, budget, path)?;
    budget.add_node(&node, path)?;
    if depth >= 128 && (value.is_object() || value.is_array()) {
        return Err(failure(
            ResolveErrorKind::LimitExceeded,
            &node,
            "default collection nesting exceeds 128",
            Some(location.clone()),
            path,
        ));
    }
    node.opaque = budget.defer_default(value, schema, node.tag.as_ref(), &node, path)?;
    // An opaque preset cannot contribute to an existing destination. Discard
    // it before expanding descendants or accumulating competing baselines.
    if node.opaque && (supplied.is_some() || scope_kind == stage::DefaultScope::Project) {
        return Ok(None);
    }
    // Keep the raw tag for opacity and supplied-tag compatibility, but a
    // projection through an already-completed ancestor cannot introduce a tag.
    if scope_kind == stage::DefaultScope::Project
        && supplied.and_then(|node| node.tag.as_ref()).is_none()
    {
        node.tag = None;
    }
    let temporal = schema.default_temporal(value);
    node.value = match value {
        JsonValue::Null => ResolvedValue::Null,
        JsonValue::Bool(value) => ResolvedValue::Bool(*value),
        JsonValue::Number(value) => {
            let raw = value.as_str();
            budget.add_bytes(&node, raw, path)?;
            checked_number(raw).map_err(|error| {
                failure(
                    ResolveErrorKind::NumericLimit,
                    &node,
                    error,
                    Some(location.clone()),
                    path,
                )
            })?;
            ResolvedValue::Number(Cow::Borrowed(value))
        }
        JsonValue::String(value) => {
            budget.add_bytes(&node, value, path)?;
            match temporal {
                Some(value) => ResolvedValue::Temporal(value),
                None => ResolvedValue::String(Cow::Borrowed(value)),
            }
        }
        JsonValue::Array(values) => ResolvedValue::Sequence(
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    from_default(
                        value,
                        schema,
                        depth + 1,
                        budget,
                        &crate::pointer::join(path, &index.to_string()),
                        None,
                        None,
                    )
                    .map(|node| node.expect("absent array items retain atomic defaults"))
                })
                .collect::<Result<_, _>>()?,
        ),

        JsonValue::Object(values) => {
            let mut fields = BTreeMap::new();
            for (key, value) in values {
                let child_path = crate::pointer::join(path, key);
                budget.add_bytes(&node, key, &child_path)?;
                let Some(child) = from_default(
                    value,
                    schema,
                    depth + 1,
                    budget,
                    &child_path,
                    supplied.and_then(|node| node.get(key)),
                    scope,
                )?
                else {
                    continue;
                };
                fields.insert(Cow::Borrowed(key.as_str()), child);
            }
            ResolvedValue::Mapping(fields)
        }
    };
    budget.classify_number(&mut node, path)?;
    Ok(Some(node))
}
fn default_tag<'a>(
    value: &'a JsonValue,
    schema: SchemaView<'a>,
    budget: &mut Budget<'_>,
    path: &str,
) -> Result<Option<ResolvedTag<'a>>, ResolveError> {
    let Some((name, binding)) = schema.default_tag(value) else {
        return Ok(None);
    };
    let location = schema.default_origin().unwrap_or(schema.location());
    let placeholder = ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin: Origin::Default(location),
        opaque: false,
        number_class: 0,
    };
    budget.add_bytes(&placeholder, name, path)?;
    if let Some(binding) = binding {
        budget.add_bytes(&placeholder, binding, path)?;
    }
    Ok(Some(ResolvedTag {
        name: Cow::Borrowed(name),
        binding: binding.map(Cow::Borrowed),
        origin: Origin::Default(location),
        supplied: false,
    }))
}

fn apply_default<'a>(
    schemas: &Schemas,
    target: &mut ResolvedNode<'a>,
    default: ResolvedNode<'a>,
    tag_owned: bool,
) {
    if target.opaque || default.opaque {
        return;
    }
    if !tag_owned && target.tag.as_ref().is_some_and(|tag| tag.supplied) {
        return;
    }
    if !matches!(
        (&target.value, &default.value),
        (ResolvedValue::Mapping(_), ResolvedValue::Mapping(_))
    ) {
        return;
    }
    if target
        .tag
        .as_ref()
        .zip(default.tag.as_ref())
        .is_some_and(|(target, default)| !same_tag_identity(schemas, target, default))
    {
        return;
    }
    if target.tag.is_none() {
        target.tag = default.tag;
    }
    if let (ResolvedValue::Mapping(target), ResolvedValue::Mapping(default)) =
        (&mut target.value, default.value)
    {
        for (key, value) in default {
            if let Some(existing) = target.get_mut(key.as_ref()) {
                apply_default(schemas, existing, value, false);
            } else {
                target.insert(key, value);
            }
        }
    }
}
fn failure(
    kind: ResolveErrorKind,
    node: &ResolvedNode<'_>,
    message: impl Into<String>,
    schema: Option<SchemaLocation>,
    path: &str,
) -> ResolveError {
    let origin = if kind == ResolveErrorKind::UnknownTag {
        node.tag.as_ref().map_or(node.origin, |tag| tag.origin)
    } else {
        node.origin
    };
    ResolveError {
        kind,
        path: path.to_owned(),
        origin: origin.into(),
        schema: schema.map(Box::new),
        message: message.into(),
    }
}

fn untagged_placeholder(origin: Origin<'_>) -> ResolvedNode<'_> {
    ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin,
        opaque: false,
        number_class: 0,
    }
}

fn same_json(left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
    equal_resolved(left, right, true) && same_number_classes(left, right)
}

fn same_number_classes(left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
    if left.number_class != right.number_class {
        return false;
    }
    match (&left.value, &right.value) {
        (ResolvedValue::Mapping(a), ResolvedValue::Mapping(b)) => a.iter().all(|(key, value)| {
            b.get(key.as_ref())
                .is_some_and(|other| same_number_classes(value, other))
        }),
        (ResolvedValue::Sequence(a), ResolvedValue::Sequence(b)) => {
            a.iter().zip(b).all(|(a, b)| same_number_classes(a, b))
        }
        _ => true,
    }
}

fn default_failure(location: &SchemaLocation, path: &str, message: &str) -> ResolveError {
    ResolveError {
        kind: ResolveErrorKind::AmbiguousDefault,
        path: path.to_owned(),
        origin: ErrorOrigin::Default(location.clone()),
        schema: Some(Box::new(location.clone())),
        message: message.to_owned(),
    }
}

fn ambiguous_completion(
    left: &ResolvedNode<'_>,
    right: &ResolvedNode<'_>,
    path: &str,
    schema: &SchemaLocation,
) -> ResolveError {
    let origin = differing_default(left, right).unwrap_or(left.origin);
    ResolveError {
        kind: ResolveErrorKind::AmbiguousDefault,
        path: path.to_owned(),
        origin: origin.into(),
        schema: Some(Box::new(schema.clone())),
        message: "valid union branches complete to different values".into(),
    }
}

fn differing_default<'a>(left: &ResolvedNode<'a>, right: &ResolvedNode<'a>) -> Option<Origin<'a>> {
    if let (ResolvedValue::Mapping(a), ResolvedValue::Mapping(b)) = (&left.value, &right.value) {
        for (key, value) in a {
            if let Some(other) = b.get(key.as_ref()) {
                if !same_json(value, other) {
                    return differing_default(value, other);
                }
            } else if matches!(value.origin, Origin::Default(_)) {
                return Some(value.origin);
            }
        }
        for (key, value) in b {
            if !a.contains_key(key.as_ref()) && matches!(value.origin, Origin::Default(_)) {
                return Some(value.origin);
            }
        }
    }
    if matches!(left.origin, Origin::Default(_)) {
        Some(left.origin)
    } else if matches!(right.origin, Origin::Default(_)) {
        Some(right.origin)
    } else {
        None
    }
}

#[cfg(test)]
mod proof_work_tests {
    use super::*;
    use crate::SchemaModule;
    use serde_json::{Map, json};

    #[test]
    fn compatible_branch_reserves_wide_schema_name_text_work() {
        check_wide_schema_name_text_work(false);
    }

    #[test]
    fn discovery_reserves_wide_schema_name_text_work() {
        check_wide_schema_name_text_work(true);
    }

    fn check_wide_schema_name_text_work(discovery: bool) {
        let prefix = "x".repeat(16 * 1024);
        let names = (0..100)
            .map(|index| format!("{prefix}{index:03}"))
            .collect::<Vec<_>>();
        // Neither proof nor discovery can admit a pass over this name text
        // within a budget smaller than its 64-byte work-unit count.
        let max_steps = names
            .iter()
            .map(|name| name.len().div_ceil(64))
            .sum::<usize>()
            / 5;
        let properties = names
            .iter()
            .map(|name| (name.clone(), json!({})))
            .collect::<Map<String, JsonValue>>();
        for required_names in [false, true] {
            let module = SchemaModule::from_value(json!({
                "types": {"app.Config": {"schema": {
                    "type": "object",
                    "properties": properties,
                    "required": if required_names { names.clone() } else { vec![] },
                    "anyOf": [{}, {"default": {}}]
                }}}
            }))
            .expect("wide-name module fits existing source admission bounds");
            let schemas = Schemas::compose([module])
                .expect("wide-name schema fits existing compilation bounds");
            let common = schemas.type_schema("app.Config").expect("root schema");
            let branch = common
                .array_child("anyOf", usize::from(!discovery))
                .expect("empty discovery arm or default-bearing compatibility arm");
            let document = crate::parse("{\n}\n").expect("empty object input");
            let mut resolver = Resolver {
                schemas: &schemas,
                budget: Budget {
                    limits: ResolutionLimits {
                        max_steps,
                        ..Default::default()
                    },
                    nodes: 0,
                    bytes: 0,
                    steps: 0,
                    stage: None,
                },
                search: CompletionSearch::default(),
                contracts: BTreeMap::new(),
                pending_defaults: None,
            };
            let node = from_input(&document, document.root(), 0, "", &mut resolver.budget)
                .expect("empty object capture needs no wide-name input work");
            let result = if discovery {
                resolver.has_implicit_branch_work(&[common], &[branch], &node, "", 0)
            } else {
                resolver.compatible_branch(&[common], branch, &node, "", true, &mut HashSet::new())
            };
            let error = result.expect_err(
                "schema-name traversal must reserve text work before exceeding its budget",
            );
            assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
        }
    }
}
