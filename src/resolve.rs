use crate::{
    Document, Node, Origin, ResolutionLimits, ResolveError, ResolveErrorKind, ResolvedDocument,
    ResolvedNode, ResolvedValue, SchemaLocation, Schemas, Value,
    json_backend::{NodeRef, equal_resolved},
    resolved::{ErrorOrigin, ResolvedTag, checked_number},
    schema::SchemaView,
};
use serde_json::Value as JsonValue;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
};

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
        let mut budget = Budget {
            limits,
            nodes: 0,
            bytes: 0,
            steps: 0,
        };
        let root = from_input(document, document.root(), 0, "", &mut budget)?;
        self.known_tags(&root, "", &mut budget)?;
        let expected_view = if let Some(expected) = expected {
            Some(self.type_schema(expected).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownExpectedType,
                    &root,
                    format!("no schema is bound to expected type {expected:?}"),
                    None,
                    "",
                )
            })?)
        } else {
            None
        };
        let mut root_views = Vec::new();
        if let Some(view) = root.tag().and_then(|tag| self.type_schema(tag)) {
            root_views.push(view);
        }
        if let Some(view) = expected_view {
            root_views.push(view);
        }
        let mut initial = Some(root);
        let mut search = CompletionSearch::default();
        let mut selected = None;
        let mut first_error = None;
        loop {
            search.start();
            // Rebuild from the original input rather than cloning a previous
            // completion. Rejected candidates must never seed later candidates.
            let root = match initial.take() {
                Some(root) => root,
                None => from_input(document, document.root(), 0, "", &mut budget)?,
            };
            let candidate = self
                .complete(
                    root,
                    &root_views,
                    "",
                    CompletionLevel::existing(0),
                    &mut budget,
                    &mut search,
                )
                .and_then(|root| {
                    self.validate_tags(&root, "", 0, &mut budget)?;
                    let asserted = root
                        .tag()
                        .and_then(|tag| self.type_schema(tag))
                        .map(|view| view.id);
                    for view in &root_views {
                        if Some(view.id) != asserted {
                            self.check_node(&root, view, "", &mut budget)?;
                        }
                    }
                    // Every selected branch must hold, not merely some branch
                    // of its enclosing union. The enclosing validators above
                    // still enforce oneOf's exact-one semantics.
                    for (path, views) in &search.branches {
                        // A missing optional property has no instance to check.
                        if let Some(node) = root.at_pointer(path) {
                            for view in views.values() {
                                self.check_node(node, view, path, &mut budget)?;
                            }
                        }
                    }
                    Ok(root)
                });
            match candidate {
                Ok(candidate) => {
                    if let Some(error) = search.conflict.take() {
                        return Err(error);
                    }
                    if let Some(prior) = &selected {
                        if !same_json(prior, &candidate) {
                            let location = search
                                .branches
                                .values()
                                .flat_map(|views| views.values())
                                .next()
                                .map(|view| view.location)
                                .expect("different completions require a union decision");
                            return Err(ambiguous_completion(prior, &candidate, "", location));
                        }
                    } else {
                        selected = Some(candidate);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind,
                        ResolveErrorKind::LimitExceeded | ResolveErrorKind::NumericLimit
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
            if !search.advance() {
                break;
            }
        }
        selected
            .map(|root| ResolvedDocument { document, root })
            .ok_or_else(|| first_error.expect("every unsuccessful candidate records its failure"))
    }

    fn complete<'a>(
        &'a self,
        mut node: ResolvedNode<'a>,
        schemas: &[SchemaView<'a>],
        path: &str,
        level: CompletionLevel,
        budget: &mut Budget,
        search: &mut CompletionSearch<'a>,
    ) -> Result<ResolvedNode<'a>, ResolveError> {
        let depth = level.depth;
        budget.step(&node, path)?;
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
        if level.apply_node_defaults && matches!(node.value, ResolvedValue::Mapping(_)) {
            // Native default annotations can introduce a tag whose schema owns
            // additional defaults. Discover every tag assertion before applying
            // payloads, so generated values never become supplied precedence.
            loop {
                schemas = self.applicable(&schemas, &node, path, budget, search)?;
                if node.tag().is_some() {
                    break;
                }
                let Some(tag) = discover_default_tag(&schemas, Some(&node), path, budget, search)?
                else {
                    break;
                };
                node.tag = Some(tag);
            }
            schemas = self.applicable(&schemas, &node, path, budget, search)?;
            let mut baseline = None;
            for schema in &schemas {
                let Some(default) = schema.value.get("default") else {
                    continue;
                };
                if !default.is_object() {
                    continue;
                }
                let candidate = from_default(default, *schema, depth, budget, path)?;
                if node.tag().is_some()
                    && candidate.tag().is_some()
                    && node.tag() != candidate.tag()
                {
                    continue;
                }
                if let Some(prior) = &mut baseline {
                    merge_candidates(prior, candidate, Some(&node), path, search);
                } else {
                    baseline = Some(candidate);
                }
            }
            if let Some(default) = baseline {
                apply_default(&mut node, default);
            }
        } else {
            schemas = self.applicable(&schemas, &node, path, budget, search)?;
        }

        if let ResolvedValue::Mapping(fields) = &mut node.value {
            for (key, child) in fields.iter_mut() {
                let child_schemas = self.property_schemas(&schemas, key);
                let child_path = join_path(path, key);
                let placeholder = ResolvedNode {
                    value: ResolvedValue::Null,
                    tag: None,
                    origin: child.origin,
                };
                let owned = std::mem::replace(child, placeholder);
                *child = self.complete(
                    owned,
                    &child_schemas,
                    &child_path,
                    CompletionLevel::existing(depth + 1),
                    budget,
                    search,
                )?;
            }
        }
        if matches!(node.value, ResolvedValue::Mapping(_)) {
            for key in self.property_names(&schemas) {
                if node.get(key).is_some() {
                    continue;
                }
                let child_schemas = self.property_schemas(&schemas, key);
                let child_path = join_path(path, key);
                if let Some(child) = self.complete_missing(
                    &child_schemas,
                    &child_path,
                    depth + 1,
                    node.origin,
                    budget,
                    search,
                )? {
                    budget.add_bytes(&node, key, &child_path)?;
                    if let ResolvedValue::Mapping(fields) = &mut node.value {
                        fields.insert(Cow::Borrowed(key), child);
                    }
                }
            }
        } else if let ResolvedValue::Sequence(items) = &mut node.value {
            for (index, child) in items.iter_mut().enumerate() {
                let item_schemas = self.items_schemas(&schemas, index);
                let child_path = join_path(path, &index.to_string());
                let placeholder = ResolvedNode {
                    value: ResolvedValue::Null,
                    tag: None,
                    origin: child.origin,
                };
                let owned = std::mem::replace(child, placeholder);
                *child = self.complete(
                    owned,
                    &item_schemas,
                    &child_path,
                    CompletionLevel::existing(depth + 1),
                    budget,
                    search,
                )?;
            }
        }
        Ok(node)
    }

    fn complete_missing<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        path: &str,
        depth: usize,
        origin: Origin<'a>,
        budget: &mut Budget,
        search: &mut CompletionSearch<'a>,
    ) -> Result<Option<ResolvedNode<'a>>, ResolveError> {
        let mut probe = untagged_placeholder(origin);
        let mut schemas = schemas.to_vec();
        loop {
            schemas = self.applicable(&schemas, &probe, path, budget, search)?;
            if probe.tag().is_some() {
                break;
            }
            let Some(tag) = discover_default_tag(&schemas, None, path, budget, search)? else {
                break;
            };
            probe.tag = Some(tag);
        }
        schemas = self.applicable(&schemas, &probe, path, budget, search)?;
        let mut completed = None;
        for schema in &schemas {
            let Some(default) = schema.value.get("default") else {
                continue;
            };
            let candidate = from_default(default, *schema, depth, budget, path)?;
            if probe.tag().is_some() && candidate.tag().is_some() && probe.tag() != candidate.tag()
            {
                continue;
            }
            if let Some(prior) = &mut completed {
                merge_candidates(prior, candidate, None, path, search);
            } else {
                completed = Some(candidate);
            }
        }
        let Some(completed) = completed else {
            return Ok(None);
        };
        self.complete(
            completed,
            &schemas,
            path,
            CompletionLevel::merged(depth),
            budget,
            search,
        )
        .map(Some)
    }

    fn conjunction<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        node: &ResolvedNode<'a>,
        path: &str,
        budget: &mut Budget,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = 0;
        for schema in schemas {
            if seen.insert(schema.id) {
                out.push(*schema);
            }
        }
        if let Some(tag) = node.tag() {
            let view = self.type_schema(tag).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownTag,
                    node,
                    format!("no schema is bound to tag {tag:?}"),
                    None,
                    path,
                )
            })?;
            if seen.insert(view.id) {
                out.push(view);
            }
        }
        while cursor < out.len() {
            let view = out[cursor];
            cursor += 1;
            budget.step(node, path)?;
            if let Some(reference) = self.referenced(view)
                && seen.insert(reference.id)
            {
                out.push(reference);
            }
            if let Some(all) = view.value.get("allOf").and_then(JsonValue::as_array) {
                for value in all {
                    if let Some(child) = self.locate(value)
                        && seen.insert(child.id)
                    {
                        out.push(child);
                    }
                }
            }
        }
        Ok(out)
    }

    fn applicable<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        node: &ResolvedNode<'a>,
        path: &str,
        budget: &mut Budget,
        search: &mut CompletionSearch<'a>,
    ) -> Result<Vec<SchemaView<'a>>, ResolveError> {
        let mut schemas = schemas.to_vec();
        let mut skipped = Vec::new();
        loop {
            schemas = self.conjunction(&schemas, node, path, budget)?;
            let Some(union) = self.next_union(&schemas, &skipped) else {
                return Ok(schemas);
            };
            budget.step(node, path)?;
            let branch = search.branch(path, union.schema.id, union.keyword, union.branches, self);
            skipped.push((union.schema.id, union.keyword));
            schemas.push(branch);
        }
    }

    fn next_union<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        skipped: &[(usize, &'static str)],
    ) -> Option<Union<'a>> {
        for schema in schemas {
            for keyword in ["anyOf", "oneOf"] {
                if skipped.contains(&(schema.id, keyword)) {
                    continue;
                }
                if let Some(branches) = schema.value.get(keyword).and_then(JsonValue::as_array) {
                    // The engine evaluates unions with no completion work.
                    if branches.iter().any(|branch| {
                        self.has_defaults(
                            self.locate(branch)
                                .expect("preflight indexed every union branch"),
                        )
                    }) {
                        return Some(Union {
                            branches,
                            schema: *schema,
                            keyword,
                        });
                    }
                }
            }
        }
        None
    }

    fn property_schemas<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        key: &str,
    ) -> Vec<SchemaView<'a>> {
        schemas
            .iter()
            .filter_map(|schema| {
                if let Some(value) = schema
                    .value
                    .get("properties")
                    .and_then(|properties| properties.get(key))
                {
                    self.locate(value)
                } else if schema.value.get("patternProperties").is_none() {
                    schema
                        .value
                        .get("additionalProperties")
                        .and_then(|value| self.locate(value))
                } else {
                    None
                }
            })
            .collect()
    }
    fn property_names<'a>(&'a self, schemas: &[SchemaView<'a>]) -> Vec<&'a str> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        for schema in schemas {
            if let Some(properties) = schema
                .value
                .get("properties")
                .and_then(JsonValue::as_object)
            {
                for key in properties.keys() {
                    if seen.insert(key.as_str()) {
                        names.push(key.as_str());
                    }
                }
            }
        }
        names
    }
    fn items_schemas<'a>(
        &'a self,
        schemas: &[SchemaView<'a>],
        index: usize,
    ) -> Vec<SchemaView<'a>> {
        schemas
            .iter()
            .filter_map(|schema| {
                let prefix = schema
                    .value
                    .get("prefixItems")
                    .and_then(JsonValue::as_array);
                let value = prefix
                    .and_then(|items| items.get(index))
                    .or_else(|| schema.value.get("items"));
                value.and_then(|value| self.locate(value))
            })
            .collect()
    }

    fn known_tags(
        &self,
        node: &ResolvedNode<'_>,
        path: &str,
        budget: &mut Budget,
    ) -> Result<(), ResolveError> {
        budget.step(node, path)?;
        if let Some(tag) = node.tag()
            && self.type_schema(tag).is_none()
        {
            return Err(failure(
                ResolveErrorKind::UnknownTag,
                node,
                format!("no schema is bound to tag {tag:?}"),
                None,
                path,
            ));
        }
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.known_tags(child, &join_path(path, key), budget)?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (index, child) in items.iter().enumerate() {
                    self.known_tags(child, &join_path(path, &index.to_string()), budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_tags(
        &self,
        node: &ResolvedNode<'_>,
        path: &str,
        depth: usize,
        budget: &mut Budget,
    ) -> Result<(), ResolveError> {
        budget.step(node, path)?;
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
            let schema = self.type_schema(tag).ok_or_else(|| {
                failure(
                    ResolveErrorKind::UnknownTag,
                    node,
                    format!("no schema is bound to tag {tag:?}"),
                    None,
                    path,
                )
            })?;
            self.check_node(node, &schema, path, budget)?;
        }
        match &node.value {
            ResolvedValue::Mapping(fields) => {
                for (key, child) in fields {
                    self.validate_tags(child, &join_path(path, key), depth + 1, budget)?;
                }
            }
            ResolvedValue::Sequence(items) => {
                for (i, child) in items.iter().enumerate() {
                    self.validate_tags(child, &join_path(path, &i.to_string()), depth + 1, budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn check_node(
        &self,
        node: &ResolvedNode<'_>,
        schema: &SchemaView<'_>,
        path: &str,
        budget: &mut Budget,
    ) -> Result<(), ResolveError> {
        budget.charge_work(
            self.evaluation_cost(*schema, resolved_work(node)),
            node,
            path,
        )?;
        if self.uses_uniqueness(*schema) {
            budget.charge_work(
                self.evaluation_cost(*schema, uniqueness_work(node)),
                node,
                path,
            )?;
        }
        self.check(NodeRef::new(node), *schema)
            .map_err(|failure_info| {
                let full_path = format!("{path}{}", failure_info.path);
                let failing = node.at_pointer(&failure_info.path).unwrap_or(node);
                let kind = if failure_info.tag {
                    ResolveErrorKind::TagMismatch
                } else {
                    ResolveErrorKind::SchemaViolation
                };
                failure(
                    kind,
                    failing,
                    failure_info.message,
                    Some(failure_info.schema),
                    &full_path,
                )
            })
    }
}

#[derive(Clone, Copy)]
struct CompletionLevel {
    depth: usize,
    apply_node_defaults: bool,
}

impl CompletionLevel {
    const fn existing(depth: usize) -> Self {
        Self {
            depth,
            apply_node_defaults: true,
        }
    }

    const fn merged(depth: usize) -> Self {
        Self {
            depth,
            apply_node_defaults: false,
        }
    }
}

struct Union<'a> {
    branches: &'a [JsonValue],
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
    branches: HashMap<String, HashMap<(usize, &'static str), SchemaView<'a>>>,
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

    fn branch(
        &mut self,
        path: &str,
        id: usize,
        keyword: &'static str,
        branches: &[JsonValue],
        schemas: &'a Schemas,
    ) -> SchemaView<'a> {
        if let Some(view) = self
            .branches
            .get(path)
            .and_then(|views| views.get(&(id, keyword)))
        {
            return *view;
        }
        let branch = schemas
            .locate(&branches[self.choose(branches.len())])
            .expect("preflight indexed every union branch");
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

fn discover_default_tag<'a>(
    schemas: &[SchemaView<'a>],
    supplied: Option<&ResolvedNode<'a>>,
    path: &str,
    budget: &mut Budget,
    search: &mut CompletionSearch<'a>,
) -> Result<Option<ResolvedTag<'a>>, ResolveError> {
    let mut discovered: Option<ResolvedTag<'a>> = None;
    for schema in schemas {
        let Some(default) = schema.value.get("default") else {
            continue;
        };
        // Existing data only merges object defaults. A tag on a skipped
        // scalar/sequence default must not leak onto a supplied mapping.
        if supplied.is_some() && !default.is_object() {
            continue;
        }
        let Some(tag) = default_tag(default, *schema, budget, path)? else {
            continue;
        };
        if let Some(prior) = &mut discovered {
            if prior.name != tag.name && search.conflicting_default_tag(&tag, path) == 1 {
                *prior = tag;
            }
        } else {
            discovered = Some(tag);
        }
    }
    Ok(discovered)
}

struct Budget {
    limits: ResolutionLimits,
    nodes: usize,
    bytes: usize,
    steps: usize,
}
impl Budget {
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
        self.bytes = self.bytes.saturating_add(text.len());
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
}

fn from_input<'a>(
    document: &'a Document,
    node: &'a Node,
    depth: usize,
    path: &str,
    budget: &mut Budget,
) -> Result<ResolvedNode<'a>, ResolveError> {
    let origin = Origin::Input(node.span);
    let mut result = ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin,
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
            origin: Origin::Input(node.tag.expect("parser tag name has its span")),
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
        Value::Sequence(items) => ResolvedValue::Sequence(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    from_input(
                        document,
                        item,
                        depth + 1,
                        &join_path(path, &index.to_string()),
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
                let child_path = join_path(path, key);
                budget.add_bytes(&result, key, &child_path)?;
                let value = from_input(document, &entry.value, depth + 1, &child_path, budget)?;
                fields.insert(Cow::Borrowed(key), value);
            }
            ResolvedValue::Mapping(fields)
        }
    };
    Ok(result)
}

fn from_default<'a>(
    value: &'a JsonValue,
    schema: SchemaView<'a>,
    depth: usize,
    budget: &mut Budget,
    path: &str,
) -> Result<ResolvedNode<'a>, ResolveError> {
    let location = schema.default_origin.unwrap_or(schema.location);
    let origin = Origin::Default(location);
    let mut node = ResolvedNode {
        value: ResolvedValue::Null,
        tag: default_tag(value, schema, budget, path)?,
        origin,
    };
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
            ResolvedValue::String(Cow::Borrowed(value))
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
                        &join_path(path, &index.to_string()),
                    )
                })
                .collect::<Result<_, _>>()?,
        ),

        JsonValue::Object(values) => {
            let mut fields = BTreeMap::new();
            for (key, value) in values {
                let child_path = join_path(path, key);
                budget.add_bytes(&node, key, &child_path)?;
                fields.insert(
                    Cow::Borrowed(key.as_str()),
                    from_default(value, schema, depth + 1, budget, &child_path)?,
                );
            }
            ResolvedValue::Mapping(fields)
        }
    };
    Ok(node)
}
fn default_tag<'a>(
    value: &'a JsonValue,
    schema: SchemaView<'a>,
    budget: &mut Budget,
    path: &str,
) -> Result<Option<ResolvedTag<'a>>, ResolveError> {
    let Some(name) = schema.default_tag(value) else {
        return Ok(None);
    };
    let location = schema.default_origin.unwrap_or(schema.location);
    let placeholder = ResolvedNode {
        value: ResolvedValue::Null,
        tag: None,
        origin: Origin::Default(location),
    };
    budget.add_bytes(&placeholder, name, path)?;
    Ok(Some(ResolvedTag {
        name: Cow::Borrowed(name),
        origin: Origin::Default(location),
    }))
}

fn merge_candidates<'a>(
    target: &mut ResolvedNode<'a>,
    mut incoming: ResolvedNode<'a>,
    supplied: Option<&ResolvedNode<'a>>,
    path: &str,
    search: &mut CompletionSearch<'a>,
) {
    if let Some(supplied) = supplied {
        if !matches!(supplied.value, ResolvedValue::Mapping(_)) {
            return;
        }
        if let Some(tag) = supplied.tag() {
            if incoming.tag().is_some_and(|incoming| incoming != tag) {
                return;
            }
            if target.tag().is_some_and(|current| current != tag) {
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
    let both_maps = matches!(
        (&target.value, &incoming.value),
        (ResolvedValue::Mapping(_), ResolvedValue::Mapping(_))
    );
    if !both_maps {
        if !same_json(target, &incoming) && search.conflicting_default(&incoming, path) == 1 {
            *target = incoming;
        }
        return;
    }
    if supplied.and_then(ResolvedNode::tag).is_none()
        && target.tag().is_some()
        && incoming.tag().is_some()
        && target.tag() != incoming.tag()
    {
        if search.conflicting_default(&incoming, path) == 1 {
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
                merge_candidates(
                    existing,
                    value,
                    supplied.and_then(|node| node.get(&key)),
                    &join_path(path, &key),
                    search,
                );
            } else {
                target.insert(key, value);
            }
        }
    }
}

fn apply_default<'a>(target: &mut ResolvedNode<'a>, default: ResolvedNode<'a>) {
    if !matches!(
        (&target.value, &default.value),
        (ResolvedValue::Mapping(_), ResolvedValue::Mapping(_))
    ) {
        return;
    }
    if target.tag().is_some() && default.tag().is_some() && target.tag() != default.tag() {
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
                apply_default(existing, value);
            } else {
                target.insert(key, value);
            }
        }
    }
}
fn join_path(parent: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}
fn failure(
    kind: ResolveErrorKind,
    node: &ResolvedNode<'_>,
    message: impl Into<String>,
    schema: Option<SchemaLocation>,
    path: &str,
) -> ResolveError {
    let origin = if matches!(
        kind,
        ResolveErrorKind::UnknownTag | ResolveErrorKind::TagMismatch
    ) {
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
    }
}

fn same_json(left: &ResolvedNode<'_>, right: &ResolvedNode<'_>) -> bool {
    equal_resolved(left, right, true)
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
