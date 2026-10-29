use crate::{
    Document, Node, Origin, ResolutionLimits, ResolveError, ResolveErrorKind, ResolvedDocument,
    ResolvedNode, ResolvedValue, SchemaLocation, Schemas, SemanticNode, SemanticValue, Value,
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
    pub(crate) fn complete_partial_default<'a>(
        &'a self,
        expected: &str,
        node: ResolvedNode<'a>,
        limits: &mut ResolutionLimits,
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
            views.push(self.schemas.type_schema(expected).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownExpectedType,
                    root,
                    format!("no schema is bound to expected type {expected:?}"),
                    None,
                    "",
                )
            })?);
        }
        Ok(views)
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
                                .flat_map(|views| views.values())
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
        self.admit_tags(&mut root, "", 0, true)?;
        for view in root_views {
            if Some(view.id()) != asserted {
                Self::check_node(&root, view, "", &mut self.budget)?;
            }
        }
        // Every selected branch must hold; enclosing validators still enforce
        // oneOf's exact-one semantics.
        for (path, views) in &self.search.branches {
            if let Some(node) = root.at_pointer(path) {
                for view in views.values() {
                    Self::check_node(node, view, path, &mut self.budget)?;
                }
            } else if self.budget.stage.is_some() {
                return Err(failure(
                    ResolveErrorKind::SchemaViolation,
                    &root,
                    "a selected-branch obligated value is missing",
                    None,
                    path,
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
            .flat_map(|views| views.values().copied())
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
                let child_schemas = Self::property_schemas(&schemas, key);
                let child_path = crate::pointer::join(path, key);
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
            for key in Self::property_names(&schemas) {
                if node.get(key).is_some() {
                    continue;
                }
                let child_schemas = Self::property_schemas(&schemas, key);
                let child_path = crate::pointer::join(path, key);
                let required = schemas.iter().any(|schema| schema.requires_property(key));
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
                schemas = self.applicable(&schemas, node, path, false)?;
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
            schemas = self.applicable(&schemas, node, path, false)?;
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
                let Some(default) = schema.value().get("default") else {
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
                    let Some(default) = schema.value().get("default") else {
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
            schemas = self.applicable(&schemas, node, path, false)?;
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
            schemas = self.applicable(&schemas, &probe, path, required)?;
            if probe.tag().is_some() {
                break;
            }
            let Some(tag) = self.discover_default_tag(&schemas, None, path)? else {
                break;
            };
            probe.tag = Some(tag);
        }
        schemas = self.applicable(&schemas, &probe, path, required)?;
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
            let Some(default) = schema.value().get("default") else {
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
                let Some(default) = schema.value().get("default") else {
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
            return self
                .complete(implicit, &schemas, path, CompletionLevel::merged(depth))
                .map(Some);
        }
        let completed = completed.expect("checked as present");
        self.complete(completed, &schemas, path, CompletionLevel::merged(depth))
            .map(Some)
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
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut schemas = schemas.to_vec();
        let mut skipped = Vec::new();
        loop {
            schemas = self.conjunction(&schemas, node, path)?;
            let Some(union) = self.next_union(&schemas, &skipped, node, path, missing)? else {
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
    ) -> Result<Option<Union<'a>>, ResolveError> {
        for schema in schemas {
            for keyword in ["anyOf", "oneOf"] {
                if skipped.contains(&(schema.id(), keyword)) {
                    continue;
                }
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
                    {
                        return Ok(Some(Union {
                            schema: *schema,
                            keyword,
                        }));
                    }
                    if self.has_implicit_branch_work(schemas, &[branch], node, path)? {
                        return Ok(Some(Union {
                            schema: *schema,
                            keyword,
                        }));
                    }
                }
            }
        }
        Ok(None)
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
        self.budget.charge_work(schemas.len(), node, path)?;
        if !schemas.iter().any(|schema| schema.requires_property(key)) {
            return Ok(0);
        }
        let mut shapes = 0;
        let mut seen = HashSet::new();
        for schema in schemas {
            self.budget.step(node, path)?;
            if let Some(property) = schema.property(key) {
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
    ) -> Result<bool, ResolveError> {
        self.budget.step(node, path)?;
        let common = self.conjunction(common, node, path)?;
        let branch = self.schema_conjunction(branch.iter().copied(), node, path)?;
        if branch.is_empty() {
            return Ok(false);
        }
        let mut combined = common.clone();
        combined.extend_from_slice(&branch);
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                for schema in &combined {
                    let properties = schema
                        .value()
                        .get("properties")
                        .and_then(JsonValue::as_object)
                        .map_or(0, serde_json::Map::len);
                    self.budget
                        .charge_work(properties.saturating_add(1), node, path)?;
                }
                for key in Self::property_names(&combined) {
                    self.budget.step(node, path)?;
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
                    self.budget.charge_work(branch.len(), node, path)?;
                    let branch_children = Self::property_schemas(&branch, key);
                    if branch_children.is_empty() {
                        continue;
                    }
                    self.budget.charge_work(common.len(), node, path)?;
                    let common_children = Self::property_schemas(&common, key);
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
                        )? {
                            return Ok(true);
                        }
                    }
                }
                for (key, value) in fields {
                    self.budget.step(value, path)?;
                    self.budget.charge_work(branch.len(), value, path)?;
                    let branch_children = Self::property_schemas(&branch, key);
                    if branch_children.is_empty() {
                        continue;
                    }
                    self.budget.charge_work(common.len(), value, path)?;
                    let common_children = Self::property_schemas(&common, key);
                    if self.has_implicit_branch_work(
                        &common_children,
                        &branch_children,
                        value,
                        &crate::pointer::join(path, key),
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
                    if self.has_implicit_branch_work(&combined, &[nested], node, path)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn property_schemas(schemas: &[SchemaView<'a>], key: &str) -> Vec<SchemaView<'a>> {
        schemas
            .iter()
            .filter_map(|schema| schema.property(key))
            .collect()
    }

    fn property_names(schemas: &[SchemaView<'a>]) -> impl Iterator<Item = &'a str> {
        let mut seen = HashSet::new();
        schemas
            .iter()
            .flat_map(|schema| schema.property_names())
            .filter(move |name| seen.insert(*name))
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
        check_constraints: bool,
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
            if check_constraints {
                Self::check_node(node, &schema, path, &mut self.budget)?;
            }
            let tag = node.tag.as_ref().expect("checked asserted tag is present");
            if tag.binding.is_none() || !matches!(tag.origin, Origin::Semantic) {
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
                        check_constraints,
                    )?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (i, child) in items.iter_mut().enumerate() {
                    self.admit_tags(
                        child,
                        &crate::pointer::join(path, &i.to_string()),
                        depth + 1,
                        check_constraints,
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
        budget.charge_work(schema.evaluation_cost(resolved_work(node)), node, path)?;
        if schema.uses_uniqueness() {
            budget.charge_work(schema.evaluation_cost(uniqueness_work(node)), node, path)?;
        }
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
    fn discover_default_tag(
        &mut self,
        schemas: &[SchemaView<'a>],
        supplied: Option<&ResolvedNode<'a>>,
        path: &str,
    ) -> Result<Option<ResolvedTag<'a>>, ResolveError> {
        let mut discovered: Option<ResolvedTag<'a>> = None;
        for schema in schemas {
            let Some(default) = schema.value().get("default") else {
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
}

fn resolved_work(node: &ResolvedNode<'_>) -> usize {
    match &node.value {
        ResolvedValue::Sequence(items) => items.iter().fold(1usize, |count, child| {
            count.saturating_add(resolved_work(child))
        }),
        ResolvedValue::Mapping(fields) => fields.iter().fold(1usize, |count, (key, child)| {
            count
                .saturating_add(key.len().div_ceil(64))
                .saturating_add(resolved_work(child))
        }),
        ResolvedValue::String(value) => 1usize.saturating_add(value.len().div_ceil(64)),
        ResolvedValue::Temporal(value) => 1usize.saturating_add(value.as_str().len().div_ceil(64)),
        ResolvedValue::Number(value) => {
            1usize.saturating_add(value.as_ref().as_str().len().div_ceil(64))
        }
        _ => 1,
    }
}

fn uniqueness_work(node: &ResolvedNode<'_>) -> usize {
    match &node.value {
        ResolvedValue::Sequence(items) => {
            let item_work = items.iter().map(resolved_work).max().unwrap_or(0);
            let pairs = items.len().saturating_mul(items.len().saturating_sub(1)) / 2;
            items
                .iter()
                .fold(pairs.saturating_mul(item_work), |work, item| {
                    work.saturating_add(uniqueness_work(item))
                })
        }
        ResolvedValue::Mapping(fields) => fields.values().fold(0usize, |work, child| {
            work.saturating_add(uniqueness_work(child))
        }),
        _ => 0,
    }
}

/// A depth-first, mixed-radix search through branch/default choices. A changed
/// choice truncates its suffix, so every replay reaches the same choice prefix
/// even when the next branch has a different shape. All replays share Budget.
#[derive(Default)]
struct CompletionSearch<'a> {
    choices: Vec<usize>,
    arities: Vec<usize>,
    branches: HashMap<String, HashMap<(SchemaId, &'static str), SchemaView<'a>>>,
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
            .and_then(|views| views.get(&(id, keyword)))
        {
            return *view;
        }
        let selected = self.choose(union.schema.array_children(keyword).len());
        let branch = union
            .schema
            .array_child(keyword, selected)
            .expect("search selects an existing union branch");
        self.branches
            .entry(path.to_owned())
            .or_default()
            .insert((id, keyword), branch);
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
                        from_semantic(
                            child,
                            depth + 1,
                            &crate::pointer::join(path, &index.to_string()),
                            budget,
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
                if !crate::names::valid_key(key) {
                    return Err(failure(
                        ResolveErrorKind::InvalidKey,
                        &result,
                        "semantic mapping key violates the DAML key grammar",
                        None,
                        &child_path,
                    ));
                }
                let child = from_semantic(child, depth + 1, &child_path, budget)?;
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
