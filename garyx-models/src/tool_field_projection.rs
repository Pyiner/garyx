use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolKind {
    Command,
    FileRead,
    FileWrite,
    FileEdit,
    Search,
    Web,
    Agent,
    Task,
    Image,
    System,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldRoot {
    Content,
    Input,
    Result,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldFormat {
    Text,
    Code,
    Path,
    Json,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldLabel {
    Call,
    Command,
    File,
    Query,
    Url,
    Prompt,
    Parameters,
    Content,
    Output,
    Result,
    Response,
    Image,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolVisibility {
    Normal,
    Nested,
    Quiet,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolValueSelector {
    pub root: RenderToolFieldRoot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolFieldSelector {
    #[serde(flatten)]
    pub value: RenderToolValueSelector,
    pub format: RenderToolFieldFormat,
    pub label: RenderToolFieldLabel,
}

impl std::ops::Deref for RenderToolFieldSelector {
    type Target = RenderToolValueSelector;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolDiffSource {
    ToolUse,
    ToolResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderToolDiffPair {
    old: Option<RenderToolValueSelector>,
    new: Option<RenderToolValueSelector>,
}

impl RenderToolDiffPair {
    pub fn old_new(old: RenderToolValueSelector, new: RenderToolValueSelector) -> Self {
        Self {
            old: Some(old),
            new: Some(new),
        }
    }

    pub fn insert(new: RenderToolValueSelector) -> Self {
        Self {
            old: None,
            new: Some(new),
        }
    }

    pub fn delete(old: RenderToolValueSelector) -> Self {
        Self {
            old: Some(old),
            new: None,
        }
    }

    pub fn old(&self) -> Option<&RenderToolValueSelector> {
        self.old.as_ref()
    }

    pub fn new(&self) -> Option<&RenderToolValueSelector> {
        self.new.as_ref()
    }

    fn checked(
        old: Option<RenderToolValueSelector>,
        new: Option<RenderToolValueSelector>,
    ) -> Option<Self> {
        (old.is_some() || new.is_some()).then_some(Self { old, new })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderToolDiffSegment {
    Unified(RenderToolValueSelector),
    Pair(RenderToolDiffPair),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderToolDiffRecipe {
    source: RenderToolDiffSource,
    segments: Vec<RenderToolDiffSegment>,
}

impl RenderToolDiffRecipe {
    pub fn new(source: RenderToolDiffSource, segments: Vec<RenderToolDiffSegment>) -> Option<Self> {
        (!segments.is_empty()).then_some(Self { source, segments })
    }

    pub fn source(&self) -> RenderToolDiffSource {
        self.source
    }

    pub fn segments(&self) -> &[RenderToolDiffSegment] {
        &self.segments
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawRenderToolDiffSegment {
    Unified {
        text: RenderToolValueSelector,
    },
    Pair {
        #[serde(default)]
        old: Option<RenderToolValueSelector>,
        #[serde(default)]
        new: Option<RenderToolValueSelector>,
    },
}

impl From<&RenderToolDiffSegment> for RawRenderToolDiffSegment {
    fn from(segment: &RenderToolDiffSegment) -> Self {
        match segment {
            RenderToolDiffSegment::Unified(text) => Self::Unified { text: text.clone() },
            RenderToolDiffSegment::Pair(pair) => Self::Pair {
                old: pair.old.clone(),
                new: pair.new.clone(),
            },
        }
    }
}

impl TryFrom<RawRenderToolDiffSegment> for RenderToolDiffSegment {
    type Error = &'static str;

    fn try_from(segment: RawRenderToolDiffSegment) -> Result<Self, Self::Error> {
        match segment {
            RawRenderToolDiffSegment::Unified { text } => Ok(Self::Unified(text)),
            RawRenderToolDiffSegment::Pair { old, new } => RenderToolDiffPair::checked(old, new)
                .map(Self::Pair)
                .ok_or("a diff pair must carry at least one side"),
        }
    }
}

impl Serialize for RenderToolDiffSegment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        RawRenderToolDiffSegment::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RenderToolDiffSegment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRenderToolDiffSegment::deserialize(deserializer)?;
        Self::try_from(raw).map_err(D::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
struct RawRenderToolDiffRecipe {
    source: RenderToolDiffSource,
    segments: Vec<RawRenderToolDiffSegment>,
}

impl From<&RenderToolDiffRecipe> for RawRenderToolDiffRecipe {
    fn from(recipe: &RenderToolDiffRecipe) -> Self {
        Self {
            source: recipe.source,
            segments: recipe
                .segments
                .iter()
                .map(RawRenderToolDiffSegment::from)
                .collect(),
        }
    }
}

impl TryFrom<RawRenderToolDiffRecipe> for RenderToolDiffRecipe {
    type Error = &'static str;

    fn try_from(recipe: RawRenderToolDiffRecipe) -> Result<Self, Self::Error> {
        let segments = recipe
            .segments
            .into_iter()
            .map(RenderToolDiffSegment::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(recipe.source, segments).ok_or("a diff recipe must carry at least one segment")
    }
}

impl Serialize for RenderToolDiffRecipe {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        RawRenderToolDiffRecipe::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RenderToolDiffRecipe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRenderToolDiffRecipe::deserialize(deserializer)?;
        Self::try_from(raw).map_err(D::Error::custom)
    }
}

/// Lightweight server-owned field mapping for one paired tool activity.
///
/// The projection deliberately carries selectors rather than selected values:
/// command output can be very large and is already available in the committed
/// message body referenced by `RenderToolEntry`. Mac and iOS therefore share
/// one provider/tool rule set without duplicating stdout in every render frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolFieldProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub kind: RenderToolKind,
    pub visibility: RenderToolVisibility,
    /// Optional concise label for the collapsed tool row. `call` remains the
    /// substantive value shown when the activity is expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RenderToolFieldSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<RenderToolFieldSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<RenderToolDiffRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<RenderToolFieldSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl RenderToolFieldProjection {
    pub(crate) fn from_message(message: &Map<String, Value>, is_result: bool) -> Option<Self> {
        let envelope_tool_name = tool_name(message);
        let visibility = tool_visibility(envelope_tool_name.as_deref());
        let tool_name = display_tool_name(message, envelope_tool_name);
        let kind = classify_tool(tool_name.as_deref());
        let special_analysis = analyze_special_shapes(message, kind, is_result);
        let diff_source = if is_result {
            RenderToolDiffSource::ToolResult
        } else {
            RenderToolDiffSource::ToolUse
        };
        let mut projection = Self {
            tool_name,
            kind,
            visibility,
            summary: None,
            call: None,
            diff: special_analysis
                .as_ref()
                .and_then(|analysis| analysis.recipe(diff_source)),
            result: None,
            status: None,
            exit_code: None,
            duration_ms: None,
        };
        if is_result {
            if kind == RenderToolKind::Image {
                // Native image-generation items only expose the revised prompt
                // on completion. Keep it as the call-side detail while the
                // generated path remains the result-side image selector.
                projection.call = call_selector(message, kind, special_analysis.as_ref());
            }
            projection.result = result_selector(message, kind, special_analysis.as_ref());
            projection.summary = call_summary_selector(message, kind, projection.call.as_ref())
                .filter(|summary| projection.call.as_ref() != Some(summary));
        } else if matches!(kind, RenderToolKind::FileWrite | RenderToolKind::FileEdit)
            && special_analysis
                .as_ref()
                .is_some_and(SpecialShapeAnalysis::owns_file_change_slots)
        {
            projection.summary = file_path_summary_selector(message)
                .or_else(|| call_summary_selector(message, kind, None));
        } else {
            projection.call = call_selector(message, kind, special_analysis.as_ref());
            projection.summary = call_summary_selector(message, kind, projection.call.as_ref())
                .filter(|summary| projection.call.as_ref() != Some(summary));
        }
        projection.absorb_metadata(message);

        (projection.tool_name.is_some()
            || projection.summary.is_some()
            || projection.call.is_some()
            || projection.diff.is_some()
            || projection.result.is_some()
            || projection.status.is_some()
            || projection.exit_code.is_some()
            || projection.duration_ms.is_some())
        .then_some(projection)
    }

    pub(crate) fn absorb_result(&mut self, result: Self) {
        if self.tool_name.is_none() {
            self.tool_name = result.tool_name;
        }
        if self.kind == RenderToolKind::Generic && result.kind != RenderToolKind::Generic {
            self.kind = result.kind;
        }
        if self.visibility == RenderToolVisibility::Normal
            && result.visibility != RenderToolVisibility::Normal
        {
            self.visibility = result.visibility;
        }
        if self.summary.is_none() {
            self.summary = result.summary;
        }
        if self.call.is_none() {
            self.call = result.call;
        }
        if self.diff.is_none() {
            self.diff = result.diff;
        }
        if let Some(result_selector) = result.result {
            let repeats_visual_call = self.call.as_ref().is_some_and(|call_selector| {
                call_selector.format == RenderToolFieldFormat::Image
                    && call_selector.value.root == result_selector.value.root
                    && call_selector.value.path == result_selector.value.path
                    && call_selector.format == result_selector.format
            });
            if !repeats_visual_call {
                self.result = Some(result_selector);
            }
        }
        if result.status.is_some() {
            self.status = result.status;
        }
        if result.exit_code.is_some() {
            self.exit_code = result.exit_code;
        }
        if result.duration_ms.is_some() {
            self.duration_ms = result.duration_ms;
        }
    }

    fn absorb_metadata(&mut self, message: &Map<String, Value>) {
        let Some(payload) = message_payload_object(message) else {
            return;
        };
        self.status = string_field(payload, &["status"]);
        self.exit_code = integer_field(payload, &["exitCode", "exit_code"]);
        self.duration_ms = unsigned_field(payload, &["durationMs", "duration_ms"]);
    }
}

pub(crate) fn merge_tool_result_projection(
    existing: Option<RenderToolFieldProjection>,
    result: Option<RenderToolFieldProjection>,
) -> Option<RenderToolFieldProjection> {
    match (existing, result) {
        (Some(mut existing), Some(result)) => {
            existing.absorb_result(result);
            Some(existing)
        }
        (Some(existing), None) => Some(existing),
        (None, Some(result)) => Some(result),
        (None, None) => None,
    }
}

fn tool_name(message: &Map<String, Value>) -> Option<String> {
    string_field(message, &["tool_name", "toolName"])
        .or_else(|| {
            message
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| string_field(metadata, &["item_type", "itemType"]))
        })
        .or_else(|| {
            message_payload_object(message).and_then(|payload| {
                string_field(payload, &["tool", "name", "tool_name", "toolName", "type"])
            })
        })
}

fn display_tool_name(
    message: &Map<String, Value>,
    envelope_tool_name: Option<String>,
) -> Option<String> {
    let item_name = compact_name(envelope_tool_name.as_deref().unwrap_or_default());
    if matches!(item_name.as_str(), "mcptoolcall" | "dynamictoolcall") {
        return message_payload_object(message)
            .and_then(|payload| string_field(payload, &["tool", "name"]))
            .or(envelope_tool_name);
    }
    envelope_tool_name
}

fn classify_tool(tool_name: Option<&str>) -> RenderToolKind {
    let name = compact_name(tool_name.unwrap_or_default());
    if matches!(
        name.as_str(),
        "contextcompaction"
            | "hookprompt"
            | "reasoning"
            | "enteredreviewmode"
            | "exitedreviewmode"
            | "enterplanmode"
            | "exitplanmode"
            | "plan"
    ) {
        return RenderToolKind::System;
    }
    if name.contains("imagegeneration")
        || name.contains("imageview")
        || name.contains("imagegen")
        || name == "viewimage"
    {
        return RenderToolKind::Image;
    }
    if name == "commandstatus" {
        return RenderToolKind::Task;
    }
    if matches!(
        name.as_str(),
        "bash"
            | "shell"
            | "command"
            | "execcommand"
            | "commandexecution"
            | "runcommand"
            | "monitor"
    ) || name.ends_with("commandexecution")
        || name.contains("executecommand")
    {
        return RenderToolKind::Command;
    }
    if name == "filechange"
        || name.contains("applypatch")
        || name.contains("multiedit")
        || name.contains("notebookedit")
        || name.contains("replacefilecontent")
        || name.contains("deletefile")
        || name.contains("renamefile")
        || name.contains("movefile")
        || name == "edit"
    {
        return RenderToolKind::FileEdit;
    }
    if matches!(name.as_str(), "write" | "create") || name.contains("writefile") {
        return RenderToolKind::FileWrite;
    }
    if matches!(name.as_str(), "read" | "view" | "open" | "cat" | "viewfile")
        || name.contains("readfile")
        || name.contains("notebookread")
    {
        return RenderToolKind::FileRead;
    }
    if name == "webrun"
        || name.contains("websearch")
        || name.contains("searchweb")
        || name.contains("webfetch")
    {
        return RenderToolKind::Web;
    }
    if matches!(
        name.as_str(),
        "grep" | "glob" | "find" | "rg" | "toolsearch"
    ) || name.starts_with("findby")
        || name.starts_with("grep")
        || name.starts_with("glob")
        || name.ends_with("search")
    {
        return RenderToolKind::Search;
    }
    if name == "agent" || name.starts_with("collabagent") || name.starts_with("subagent") {
        return RenderToolKind::Agent;
    }
    if name.starts_with("task")
        || matches!(
            name.as_str(),
            "todowrite" | "managetask" | "schedule" | "sleep"
        )
    {
        return RenderToolKind::Task;
    }
    RenderToolKind::Generic
}

fn tool_visibility(tool_name: Option<&str>) -> RenderToolVisibility {
    let name = compact_name(tool_name.unwrap_or_default());
    if matches!(
        name.as_str(),
        "contextcompaction" | "hookprompt" | "reasoning"
    ) {
        RenderToolVisibility::Hidden
    } else if name.starts_with("subagent") {
        RenderToolVisibility::Nested
    } else if matches!(
        name.as_str(),
        "plan" | "enteredreviewmode" | "exitedreviewmode" | "enterplanmode" | "exitplanmode"
    ) {
        RenderToolVisibility::Quiet
    } else {
        RenderToolVisibility::Normal
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SpecialShapeAnalysis {
    Unsupported,
    ComposableEmpty,
    Composable {
        segments: Vec<RenderToolDiffSegment>,
        consumed: HashSet<RenderToolValueSelector>,
    },
}

impl SpecialShapeAnalysis {
    fn recipe(&self, source: RenderToolDiffSource) -> Option<RenderToolDiffRecipe> {
        let Self::Composable { segments, .. } = self else {
            return None;
        };
        RenderToolDiffRecipe::new(source, segments.clone())
    }

    fn consumed(&self) -> Option<&HashSet<RenderToolValueSelector>> {
        match self {
            Self::Composable { consumed, .. } => Some(consumed),
            Self::Unsupported | Self::ComposableEmpty => None,
        }
    }

    fn suppresses_whole_object_fallback(&self) -> bool {
        matches!(self, Self::Composable { .. })
    }

    fn owns_file_change_slots(&self) -> bool {
        matches!(self, Self::Composable { .. } | Self::ComposableEmpty)
    }
}

#[derive(Default)]
struct SpecialShapeGroup {
    segments: Vec<RenderToolDiffSegment>,
    consumed: HashSet<RenderToolValueSelector>,
}

impl SpecialShapeGroup {
    fn push_unified(&mut self, selector: RenderToolValueSelector) {
        self.consumed.insert(selector.clone());
        self.segments.push(RenderToolDiffSegment::Unified(selector));
    }

    fn push_pair(
        &mut self,
        old: Option<RenderToolValueSelector>,
        new: Option<RenderToolValueSelector>,
    ) {
        let pair = match (old, new) {
            (Some(old), Some(new)) => RenderToolDiffPair::old_new(old, new),
            (Some(old), None) => RenderToolDiffPair::delete(old),
            (None, Some(new)) => RenderToolDiffPair::insert(new),
            (None, None) => return,
        };
        if let Some(old) = pair.old() {
            self.consumed.insert(old.clone());
        }
        if let Some(new) = pair.new() {
            self.consumed.insert(new.clone());
        }
        self.segments.push(RenderToolDiffSegment::Pair(pair));
    }
}

fn analyze_special_shapes(
    message: &Map<String, Value>,
    kind: RenderToolKind,
    is_result: bool,
) -> Option<SpecialShapeAnalysis> {
    let (root, prefix, object) = derivation_object(message, is_result)?;
    let mut candidate_present = false;
    let mut pre_rendered = SpecialShapeGroup::default();

    // `changes` and `diff` are one ordered group. The entire pass is still
    // grammar-checked before this group can win over structured shapes.
    for key in ["changes", "diff"] {
        let Some(value) = special_candidate(object, key) else {
            continue;
        };
        candidate_present = true;
        let selector = value_selector(root, &prefix, key);
        if parse_unified_value(value, selector, &mut pre_rendered).is_err() {
            return Some(SpecialShapeAnalysis::Unsupported);
        }
    }

    let structured =
        !is_result && matches!(kind, RenderToolKind::FileWrite | RenderToolKind::FileEdit);
    let mut direct_pair = SpecialShapeGroup::default();
    let mut edits = SpecialShapeGroup::default();
    let mut content = SpecialShapeGroup::default();

    if structured {
        let direct_present = ["old_string", "new_string"]
            .iter()
            .any(|key| special_candidate(object, key).is_some());
        if direct_present {
            candidate_present = true;
            if parse_pair_object(root, &prefix, object, &mut direct_pair).is_err() {
                return Some(SpecialShapeAnalysis::Unsupported);
            }
        }

        if let Some(value) = special_candidate(object, "edits") {
            candidate_present = true;
            let Value::Array(values) = value else {
                return Some(SpecialShapeAnalysis::Unsupported);
            };
            for (index, edit) in values.iter().enumerate() {
                if edit.is_null() {
                    continue;
                }
                let Value::Object(edit) = edit else {
                    return Some(SpecialShapeAnalysis::Unsupported);
                };
                let mut edit_prefix = prefix.clone();
                edit_prefix.extend(["edits".to_owned(), index.to_string()]);
                if parse_pair_object(root, &edit_prefix, edit, &mut edits).is_err() {
                    return Some(SpecialShapeAnalysis::Unsupported);
                }
            }
        }

        for key in ["content", "new_source"] {
            let Some(value) = special_candidate(object, key) else {
                continue;
            };
            candidate_present = true;
            let Value::String(value) = value else {
                return Some(SpecialShapeAnalysis::Unsupported);
            };
            if !value.is_empty() {
                content.push_pair(None, Some(value_selector(root, &prefix, key)));
            }
        }
    }

    if !candidate_present {
        return None;
    }
    [pre_rendered, direct_pair, edits, content]
        .into_iter()
        .find(|group| !group.segments.is_empty())
        .map(|group| SpecialShapeAnalysis::Composable {
            segments: group.segments,
            consumed: group.consumed,
        })
        .or(Some(SpecialShapeAnalysis::ComposableEmpty))
}

fn derivation_object(
    message: &Map<String, Value>,
    is_result: bool,
) -> Option<(RenderToolFieldRoot, Vec<String>, &Map<String, Value>)> {
    if is_result {
        return message
            .get("content")
            .and_then(Value::as_object)
            .map(|object| (RenderToolFieldRoot::Content, Vec::new(), object));
    }
    call_input_object(message)
}

fn special_candidate<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    object.get(key).filter(|value| !value.is_null())
}

fn value_selector(
    root: RenderToolFieldRoot,
    prefix: &[String],
    key: &str,
) -> RenderToolValueSelector {
    let mut path = prefix.to_vec();
    path.push(key.to_owned());
    RenderToolValueSelector { root, path }
}

fn parse_unified_value(
    value: &Value,
    selector: RenderToolValueSelector,
    group: &mut SpecialShapeGroup,
) -> Result<(), ()> {
    match value {
        Value::String(value) => {
            if !value.is_empty() {
                group.push_unified(selector);
            }
            Ok(())
        }
        Value::Object(object) => {
            let Some(Value::String(value)) = object.get("diff") else {
                return Err(());
            };
            if !value.is_empty() {
                let mut selector = selector;
                selector.path.push("diff".to_owned());
                group.push_unified(selector);
            }
            Ok(())
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                if value.is_null() {
                    continue;
                }
                let mut selector = selector.clone();
                selector.path.push(index.to_string());
                parse_unified_value(value, selector, group)?;
            }
            Ok(())
        }
        Value::Null => Ok(()),
        Value::Bool(_) | Value::Number(_) => Err(()),
    }
}

fn parse_pair_object(
    root: RenderToolFieldRoot,
    prefix: &[String],
    object: &Map<String, Value>,
    group: &mut SpecialShapeGroup,
) -> Result<(), ()> {
    let old = special_candidate(object, "old_string");
    let new = special_candidate(object, "new_string");
    if old.is_none() && new.is_none() {
        return Err(());
    }
    let old = match old {
        Some(Value::String(value)) if !value.is_empty() => {
            Some(value_selector(root, prefix, "old_string"))
        }
        Some(Value::String(_)) | None => None,
        Some(_) => return Err(()),
    };
    let new = match new {
        Some(Value::String(value)) if !value.is_empty() => {
            Some(value_selector(root, prefix, "new_string"))
        }
        Some(Value::String(_)) | None => None,
        Some(_) => return Err(()),
    };
    group.push_pair(old, new);
    Ok(())
}

const CALL_SUMMARY_KEYS: &[&str] = &["label", "description", "toolSummary", "toolAction"];
const LABEL_CALL_SUMMARY_KEYS: &[&str] = &["label"];

fn call_summary_keys(
    kind: RenderToolKind,
    call: Option<&RenderToolFieldSelector>,
) -> &'static [&'static str] {
    if kind == RenderToolKind::Task
        && call
            .and_then(|selector| selector.value.path.last())
            .is_some_and(|key| key == "subject")
    {
        // Preserve the legacy Task order: label, subject, description,
        // toolSummary, toolAction, then prompt/ids. Once subject wins the
        // substantive selector, only the earlier label may override it in the
        // collapsed row. Without a subject, later descriptors remain eligible
        // so command_status does not fall through to its raw CommandId.
        LABEL_CALL_SUMMARY_KEYS
    } else {
        CALL_SUMMARY_KEYS
    }
}

fn call_summary_selector(
    message: &Map<String, Value>,
    kind: RenderToolKind,
    call: Option<&RenderToolFieldSelector>,
) -> Option<RenderToolFieldSelector> {
    let (root, prefix, input) = call_input_object(message)?;
    select_object_field(
        root,
        &prefix,
        input,
        call_summary_keys(kind, call),
        kind,
        false,
        None,
    )
}

fn file_path_summary_selector(message: &Map<String, Value>) -> Option<RenderToolFieldSelector> {
    let (root, prefix, input) = call_input_object(message)?;
    select_object_field(
        root,
        &prefix,
        input,
        &[
            "file_path",
            "filePath",
            "AbsolutePath",
            "TargetFile",
            "notebook_path",
            "path",
            "file",
        ],
        RenderToolKind::FileEdit,
        false,
        None,
    )
}

fn call_selector(
    message: &Map<String, Value>,
    kind: RenderToolKind,
    analysis: Option<&SpecialShapeAnalysis>,
) -> Option<RenderToolFieldSelector> {
    let (root, prefix, input) = call_input_object(message)?;
    let keys: &[&str] = match kind {
        RenderToolKind::Command => &["cmd", "command", "CommandLine"],
        RenderToolKind::FileRead => &[
            "file_path",
            "filePath",
            "AbsolutePath",
            "TargetFile",
            "notebook_path",
            "path",
            "file",
        ],
        RenderToolKind::FileWrite | RenderToolKind::FileEdit => &[
            "file_path",
            "filePath",
            "AbsolutePath",
            "TargetFile",
            "notebook_path",
            "path",
            "file",
            "content",
        ],
        RenderToolKind::Search | RenderToolKind::Web => &["query", "pattern", "search", "url"],
        RenderToolKind::Agent => &["task", "prompt", "message", "agentPath"],
        RenderToolKind::Task => &[
            "subject",
            "Prompt",
            "prompt",
            "TaskId",
            "task_id",
            "CommandId",
            "DurationSeconds",
            "duration",
            "status",
            "Action",
            "action",
        ],
        RenderToolKind::Image => &[
            "prompt",
            "revisedPrompt",
            "revised_prompt",
            "path",
            "file_path",
            "filePath",
        ],
        RenderToolKind::System | RenderToolKind::Generic => &[
            "title",
            "subject",
            "cmd",
            "command",
            "CommandLine",
            "query",
            "pattern",
            "url",
            "file_path",
            "filePath",
            "AbsolutePath",
            "path",
            "prompt",
            "message",
            "skill",
            "TaskId",
            "task_id",
            "Action",
            "action",
            "arguments",
            "params",
        ],
    };
    let consumed = analysis.and_then(SpecialShapeAnalysis::consumed);
    select_object_field(root, &prefix, input, keys, kind, false, consumed)
        .or_else(|| {
            select_object_field(
                root,
                &prefix,
                input,
                CALL_SUMMARY_KEYS,
                kind,
                false,
                consumed,
            )
        })
        .or_else(|| {
            (kind != RenderToolKind::Image
                && !input.is_empty()
                && !analysis.is_some_and(SpecialShapeAnalysis::suppresses_whole_object_fallback))
            .then(|| RenderToolFieldSelector {
                value: RenderToolValueSelector { root, path: prefix },
                format: RenderToolFieldFormat::Json,
                label: if input.len() == 1 {
                    RenderToolFieldLabel::Call
                } else {
                    RenderToolFieldLabel::Parameters
                },
            })
        })
}

fn result_selector(
    message: &Map<String, Value>,
    kind: RenderToolKind,
    analysis: Option<&SpecialShapeAnalysis>,
) -> Option<RenderToolFieldSelector> {
    if let Some(content) = message.get("content") {
        if let Some(object) = content.as_object() {
            let keys: &[&str] = if kind == RenderToolKind::Image {
                &[
                    "savedPath",
                    "saved_path",
                    "aggregatedOutput",
                    "result",
                    "output",
                    "content",
                    "stdout",
                    "stderr",
                    "text",
                    "message",
                    "error",
                    "action",
                    "path",
                ]
            } else {
                &[
                    "aggregatedOutput",
                    "result",
                    "output",
                    "content",
                    "stdout",
                    "stderr",
                    "text",
                    "message",
                    "error",
                    "action",
                    "path",
                ]
            };
            if let Some(selector) = select_object_field(
                RenderToolFieldRoot::Content,
                &[],
                object,
                keys,
                kind,
                true,
                analysis.and_then(SpecialShapeAnalysis::consumed),
            ) {
                return Some(selector);
            }
            // A command execution with no meaningful output should have no
            // result body. Falling back to the entire execution envelope is
            // precisely the noisy JSON presentation this projection avoids.
            if matches!(kind, RenderToolKind::Command | RenderToolKind::Image) {
                return None;
            }
            if !object.is_empty()
                && !analysis.is_some_and(SpecialShapeAnalysis::suppresses_whole_object_fallback)
            {
                return Some(RenderToolFieldSelector {
                    value: RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: Vec::new(),
                    },
                    format: RenderToolFieldFormat::Json,
                    label: RenderToolFieldLabel::Result,
                });
            }
        } else if meaningful_value(content) {
            return Some(selector_for_value(
                RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: Vec::new(),
                },
                content,
                kind,
                true,
                None,
            ));
        }
    }

    for (root, key) in [
        (RenderToolFieldRoot::Result, "result"),
        (RenderToolFieldRoot::Text, "text"),
    ] {
        if let Some(value) = message.get(key).filter(|value| meaningful_value(value)) {
            return Some(selector_for_value(
                RenderToolValueSelector {
                    root,
                    path: Vec::new(),
                },
                value,
                kind,
                true,
                Some(key),
            ));
        }
    }
    None
}

fn call_input_object(
    message: &Map<String, Value>,
) -> Option<(RenderToolFieldRoot, Vec<String>, &Map<String, Value>)> {
    if let Some(content) = message.get("content").and_then(Value::as_object) {
        for key in ["input", "arguments", "args", "params"] {
            if let Some(input) = content.get(key).and_then(Value::as_object) {
                return Some((RenderToolFieldRoot::Content, vec![key.to_owned()], input));
            }
        }
        return Some((RenderToolFieldRoot::Content, Vec::new(), content));
    }
    message
        .get("input")
        .and_then(Value::as_object)
        .map(|input| (RenderToolFieldRoot::Input, Vec::new(), input))
}

fn select_object_field(
    root: RenderToolFieldRoot,
    prefix: &[String],
    object: &Map<String, Value>,
    keys: &[&str],
    kind: RenderToolKind,
    result: bool,
    excluded: Option<&HashSet<RenderToolValueSelector>>,
) -> Option<RenderToolFieldSelector> {
    keys.iter().find_map(|key| {
        let value = object.get(*key).filter(|value| meaningful_value(value))?;
        if result
            && kind == RenderToolKind::Image
            && *key == "result"
            && looks_like_base64_image_result(value)
        {
            return None;
        }
        let mut path = prefix.to_vec();
        path.push((*key).to_owned());
        let selector = RenderToolValueSelector { root, path };
        if excluded.is_some_and(|excluded| excluded.contains(&selector)) {
            return None;
        }
        Some(selector_for_value(selector, value, kind, result, Some(key)))
    })
}

fn selector_for_value(
    value_selector: RenderToolValueSelector,
    value: &Value,
    kind: RenderToolKind,
    result: bool,
    key: Option<&str>,
) -> RenderToolFieldSelector {
    let key = key.unwrap_or_default();
    let (label, format) = if key == "aggregatedOutput" || matches!(key, "stdout" | "stderr") {
        (RenderToolFieldLabel::Output, RenderToolFieldFormat::Code)
    } else if key == "error" {
        (RenderToolFieldLabel::Error, RenderToolFieldFormat::Code)
    } else if matches!(key, "command" | "cmd" | "CommandLine") {
        (RenderToolFieldLabel::Command, RenderToolFieldFormat::Code)
    } else if matches!(
        key,
        "file_path" | "filePath" | "AbsolutePath" | "TargetFile" | "notebook_path" | "file"
    ) {
        (RenderToolFieldLabel::File, RenderToolFieldFormat::Path)
    } else if matches!(key, "savedPath" | "saved_path" | "path") && kind == RenderToolKind::Image {
        (RenderToolFieldLabel::Image, RenderToolFieldFormat::Image)
    } else if key == "path" {
        (RenderToolFieldLabel::File, RenderToolFieldFormat::Path)
    } else if matches!(key, "query" | "pattern" | "search") {
        (RenderToolFieldLabel::Query, RenderToolFieldFormat::Text)
    } else if key == "url" {
        (RenderToolFieldLabel::Url, RenderToolFieldFormat::Text)
    } else if matches!(
        key,
        "prompt" | "Prompt" | "revisedPrompt" | "revised_prompt"
    ) {
        (RenderToolFieldLabel::Prompt, RenderToolFieldFormat::Text)
    } else if key == "content" && result && kind == RenderToolKind::Command {
        (RenderToolFieldLabel::Output, RenderToolFieldFormat::Code)
    } else if key == "content" {
        (RenderToolFieldLabel::Content, format_for_value(value))
    } else if result && key == "action" {
        (RenderToolFieldLabel::Response, RenderToolFieldFormat::Json)
    } else if result {
        (RenderToolFieldLabel::Result, format_for_value(value))
    } else if matches!(key, "arguments" | "params") {
        (
            RenderToolFieldLabel::Parameters,
            RenderToolFieldFormat::Json,
        )
    } else {
        (RenderToolFieldLabel::Call, format_for_value(value))
    };
    RenderToolFieldSelector {
        value: value_selector,
        format,
        label,
    }
}

fn format_for_value(value: &Value) -> RenderToolFieldFormat {
    if value_contains_image(value, 0) {
        return RenderToolFieldFormat::Image;
    }
    match value {
        Value::Object(_) | Value::Array(_) => RenderToolFieldFormat::Json,
        _ => RenderToolFieldFormat::Text,
    }
}

fn value_contains_image(value: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_image(item, depth + 1)),
        Value::Object(object) => {
            object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("image"))
                || object
                    .get("source")
                    .and_then(Value::as_object)
                    .is_some_and(|source| source.get("data").is_some())
                || object
                    .values()
                    .any(|value| value_contains_image(value, depth + 1))
        }
        _ => false,
    }
}

fn message_payload_object(message: &Map<String, Value>) -> Option<&Map<String, Value>> {
    message.get("content").and_then(Value::as_object)
}

fn meaningful_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

const LARGE_BASE64_BLOB_MIN_LENGTH: usize = 16_384;

fn looks_like_base64_image_result(value: &Value) -> bool {
    let Some(value) = value.as_str().map(str::trim) else {
        return false;
    };
    let data_url_payload = value
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .filter(|(metadata, _)| metadata.to_ascii_lowercase().contains(";base64"));
    let candidate = data_url_payload.map_or(value, |(_, payload)| payload);
    let has_image_signature = [
        "iVBORw0KGgo", // PNG
        "/9j/",        // JPEG
        "R0lGOD",      // GIF
        "UklGR",       // WebP
        "Qk",          // BMP
        "SUkq",        // little-endian TIFF
        "TU0A",        // big-endian TIFF
    ]
    .iter()
    .any(|signature| candidate.starts_with(signature));
    (data_url_payload.is_some()
        || has_image_signature
        || candidate.len() >= LARGE_BASE64_BLOB_MIN_LENGTH)
        && candidate.len().is_multiple_of(4)
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

fn string_field(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn integer_field(object: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_i64().or_else(|| value.as_u64()?.try_into().ok()))
    })
}

fn unsigned_field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()))
    })
}

fn compact_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    fn call(tool_name: &str, input: Value) -> Map<String, Value> {
        object(json!({
            "role": "tool_use",
            "tool_name": tool_name,
            "content": {
                "tool": tool_name,
                "input": input,
            },
        }))
    }

    fn direct_call(tool_name: &str, content: Value) -> Map<String, Value> {
        object(json!({
            "role": "tool_use",
            "tool_name": tool_name,
            "content": content,
        }))
    }

    fn result(tool_name: &str, content: Value) -> Map<String, Value> {
        object(json!({
            "role": "tool_result",
            "tool_name": tool_name,
            "content": content,
        }))
    }

    fn selector_path(selector: Option<&RenderToolFieldSelector>) -> Option<Vec<&str>> {
        selector.map(|selector| selector.value.path.iter().map(String::as_str).collect())
    }

    fn assert_file_change_slots(projection: &RenderToolFieldProjection, expected_path: &[&str]) {
        assert_eq!(
            projection.summary.as_ref().map(|selector| selector
                .value
                .path
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()),
            Some(expected_path.to_vec())
        );
        assert_eq!(projection.call, None);
        assert_eq!(
            projection.summary.as_ref().map(|selector| selector.format),
            Some(RenderToolFieldFormat::Path)
        );
        assert_eq!(
            projection.summary.as_ref().map(|selector| selector.label),
            Some(RenderToolFieldLabel::File)
        );
    }

    #[test]
    fn codex_command_projects_command_and_aggregated_output_without_copying_values() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "command": "/bin/zsh -lc 'git status --short'",
                "status": "inProgress"
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "aggregatedOutput": " M AGENTS.md\n M CLAUDE.md\n",
                "status": "completed",
                "exitCode": 0,
                "durationMs": 12
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.kind, RenderToolKind::Command);
        assert_eq!(projection.call.as_ref().unwrap().path, ["command"]);
        assert_eq!(
            projection.result.as_ref().unwrap().path,
            ["aggregatedOutput"]
        );
        assert_eq!(projection.exit_code, Some(0));
        assert_eq!(projection.duration_ms, Some(12));
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(!wire.contains("AGENTS.md"));
        assert!(!wire.contains("git status"));
    }

    #[test]
    fn claude_bash_projects_command_as_call_detail() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "Bash",
            "content": {
                "tool": "Bash",
                "input": {
                    "description": "Read schema definition",
                    "command": "git status --short"
                }
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "content": { "result": "clean", "text": "clean" }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.call.as_ref().unwrap().path, ["input", "command"]);
        assert_eq!(
            projection.call.as_ref().unwrap().label,
            RenderToolFieldLabel::Command
        );
        assert_eq!(
            projection.summary.as_ref().unwrap().path,
            ["input", "description"]
        );
        assert_eq!(projection.result.as_ref().unwrap().path, ["result"]);
    }

    #[test]
    fn antigravity_projects_tool_summary_and_content_despite_result_type_change() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "run_command",
            "content": {
                "name": "run_command",
                "args": {
                    "toolSummary": "\"Check status\"",
                    "CommandLine": "\"git status --short\""
                }
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "RUN_COMMAND",
            "content": {
                "type": "RUN_COMMAND",
                "status": "DONE",
                "content": " M AGENTS.md\n"
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.tool_name.as_deref(), Some("run_command"));
        assert_eq!(
            projection.call.as_ref().unwrap().path,
            ["args", "CommandLine"]
        );
        assert_eq!(
            projection.summary.as_ref().unwrap().path,
            ["args", "toolSummary"]
        );
        assert_eq!(projection.result.as_ref().unwrap().path, ["content"]);
        assert_eq!(
            projection.result.as_ref().unwrap().label,
            RenderToolFieldLabel::Output
        );
        assert_eq!(projection.status.as_deref(), Some("DONE"));
    }

    #[test]
    fn task_create_keeps_subject_ahead_of_long_description() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "TaskCreate",
            "content": {
                "tool": "TaskCreate",
                "input": {
                    "subject": "Verify projection behavior",
                    "description": "Run the full cross-platform validation and write a detailed review."
                }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.kind, RenderToolKind::Task);
        assert_eq!(projection.call.as_ref().unwrap().path, ["input", "subject"]);
        assert_eq!(projection.summary, None);
    }

    #[test]
    fn command_status_keeps_tool_summary_ahead_of_raw_command_id() {
        // Sanitized real Antigravity command_status shape.
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "command_status",
            "content": {
                "name": "command_status",
                "args": {
                    "CommandId": "synthetic/task-8",
                    "OutputCharacterCount": 128,
                    "WaitDurationSeconds": 5,
                    "toolAction": "Wait for package metadata",
                    "toolSummary": "Check status"
                }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.kind, RenderToolKind::Task);
        assert_eq!(
            projection.call.as_ref().unwrap().path,
            ["args", "CommandId"]
        );
        assert_eq!(
            projection.summary.as_ref().unwrap().path,
            ["args", "toolSummary"]
        );
    }

    #[test]
    fn command_with_null_aggregated_output_does_not_fall_back_to_json_envelope() {
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "aggregatedOutput": null,
                "command": "true",
                "status": "completed",
                "exitCode": 0
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&result, true).unwrap();

        assert_eq!(projection.result, None);
        assert_eq!(projection.status.as_deref(), Some("completed"));
        assert_eq!(projection.exit_code, Some(0));
    }

    #[test]
    fn web_search_with_empty_query_projects_parameters_without_a_summary() {
        // Sanitized real Codex webSearch start shape: the provider commits the
        // query later, so the initial call has only structured parameters.
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "webSearch",
            "content": {
                "action": null,
                "id": "exec-00000000-0000-0000-0000-000000000001",
                "query": "",
                "type": "webSearch"
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.kind, RenderToolKind::Web);
        assert_eq!(projection.summary, None);
        assert_eq!(projection.call.as_ref().unwrap().path, Vec::<String>::new());
        assert_eq!(
            projection.call.as_ref().unwrap().format,
            RenderToolFieldFormat::Json
        );
        assert_eq!(
            projection.call.as_ref().unwrap().label,
            RenderToolFieldLabel::Parameters
        );
    }

    #[test]
    fn codex_native_image_generation_projects_prompt_and_saved_image_path() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "imageGeneration",
            "metadata": {
                "item_type": "imageGeneration",
                "source": "codex_app_server"
            },
            "content": {
                "id": "exec-00000000-0000-0000-0000-000000000001",
                "result": "",
                "revisedPrompt": null,
                "status": "in_progress",
                "type": "imageGeneration"
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "imageGeneration",
            "metadata": {
                "item_type": "imageGeneration",
                "source": "codex_app_server"
            },
            "content": {
                "id": "exec-00000000-0000-0000-0000-000000000001",
                "result": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                "revisedPrompt": "A synthetic lighthouse beneath a violet evening sky.",
                "savedPath": "/Users/test/.codex/generated_images/00000000-0000-0000-0000-000000000001/exec-00000000-0000-0000-0000-000000000001.png",
                "status": "completed",
                "type": "imageGeneration"
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        let started_call = projection.call.clone();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(
            (
                started_call,
                projection.call.clone(),
                projection.result.clone()
            ),
            (
                None,
                Some(RenderToolFieldSelector {
                    value: RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec!["revisedPrompt".to_owned()],
                    },
                    format: RenderToolFieldFormat::Text,
                    label: RenderToolFieldLabel::Prompt,
                }),
                Some(RenderToolFieldSelector {
                    value: RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec!["savedPath".to_owned()],
                    },
                    format: RenderToolFieldFormat::Image,
                    label: RenderToolFieldLabel::Image,
                }),
            )
        );
        assert_eq!(projection.status.as_deref(), Some("completed"));
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(!wire.contains("iVBORw0KGgo"));
        assert!(!wire.contains("A synthetic lighthouse"));
        assert!(!wire.contains("/Users/test"));
    }

    #[test]
    fn image_generation_projects_snake_case_prompt_and_saved_path_aliases() {
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "imageGeneration",
            "content": {
                "id": "exec-snake-case",
                "result": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                "revised_prompt": "A synthetic paper boat on a quiet pond.",
                "saved_path": "/Users/test/.codex/generated_images/synthetic/exec-snake-case.png",
                "status": "completed",
                "type": "imageGeneration"
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&result, true).unwrap();

        assert_eq!(
            projection.call,
            Some(RenderToolFieldSelector {
                value: RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["revised_prompt".to_owned()],
                },
                format: RenderToolFieldFormat::Text,
                label: RenderToolFieldLabel::Prompt,
            })
        );
        assert_eq!(
            projection.result,
            Some(RenderToolFieldSelector {
                value: RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["saved_path".to_owned()],
                },
                format: RenderToolFieldFormat::Image,
                label: RenderToolFieldLabel::Image,
            })
        );
    }

    #[test]
    fn image_generation_never_projects_raw_base64_result_as_text_or_json() {
        for raw_result in [
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_owned(),
            "A".repeat(LARGE_BASE64_BLOB_MIN_LENGTH),
        ] {
            let result = object(json!({
                "role": "tool_result",
                "tool_name": "imageGeneration",
                "content": {
                    "id": "exec-no-saved-path",
                    "result": raw_result,
                    "revisedPrompt": null,
                    "status": "completed",
                    "type": "imageGeneration"
                }
            }));

            let projection = RenderToolFieldProjection::from_message(&result, true).unwrap();

            assert_eq!(projection.call, None);
            assert_eq!(projection.result, None);
            assert_eq!(projection.status.as_deref(), Some("completed"));
        }
    }

    #[test]
    fn mcp_projection_uses_inner_tool_name_and_primary_argument() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "mcpToolCall",
            "content": {
                "type": "mcpToolCall",
                "server": "garyx",
                "tool": "capsule_create",
                "arguments": { "title": "Synthetic capsule" }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.tool_name.as_deref(), Some("capsule_create"));
        assert_eq!(projection.kind, RenderToolKind::Generic);
        assert_eq!(
            projection.call.as_ref().unwrap().path,
            ["arguments", "title"]
        );
    }

    #[test]
    fn wrapped_dynamic_tool_is_classified_by_its_display_name() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "dynamicToolCall",
            "content": {
                "type": "dynamicToolCall",
                "tool": "exec_command",
                "arguments": { "cmd": "git status --short" }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.tool_name.as_deref(), Some("exec_command"));
        assert_eq!(projection.kind, RenderToolKind::Command);
        assert_eq!(projection.call.as_ref().unwrap().path, ["arguments", "cmd"]);
    }

    #[test]
    fn file_change_does_not_repeat_the_same_diff_as_its_result() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "fileChange",
            "content": {
                "type": "fileChange",
                "changes": [{ "path": "/Users/test/README.md", "diff": "+hello" }]
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "fileChange",
            "content": {
                "type": "fileChange",
                "changes": [{ "path": "/Users/test/README.md", "diff": "+hello" }],
                "status": "completed"
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.call, None);
        assert_eq!(projection.diff.as_ref().unwrap().segments().len(), 1);
        assert_eq!(projection.result, None);
        assert_eq!(projection.status.as_deref(), Some("completed"));
    }

    #[test]
    fn captured_claude_file_change_shapes_project_selector_only_recipes() {
        // Sanitized from committed Claude Code transcript shapes. Values are
        // deliberately synthetic; only their provider field layout is real.
        let edit = call(
            "Edit",
            json!({
                "file_path": "/Users/test/repo/Sample.swift",
                "old_string": "let oldValue = 1",
                "new_string": "let newValue = 2",
                "replace_all": false,
            }),
        );
        let mut edit_projection = RenderToolFieldProjection::from_message(&edit, false).unwrap();
        assert_eq!(edit_projection.kind, RenderToolKind::FileEdit);
        assert_file_change_slots(&edit_projection, &["input", "file_path"]);
        assert_eq!(
            serde_json::to_value(edit_projection.diff.as_ref().unwrap()).unwrap(),
            json!({
                "source": "tool_use",
                "segments": [{
                    "pair": {
                        "old": {"root": "content", "path": ["input", "old_string"]},
                        "new": {"root": "content", "path": ["input", "new_string"]},
                    }
                }]
            })
        );
        edit_projection.absorb_result(
            RenderToolFieldProjection::from_message(
                &result("Edit", json!({"result": "Updated synthetic file"})),
                true,
            )
            .unwrap(),
        );
        assert_file_change_slots(&edit_projection, &["input", "file_path"]);
        assert_eq!(
            selector_path(edit_projection.result.as_ref()),
            Some(vec!["result"])
        );

        let write = call(
            "Write",
            json!({
                "file_path": "/Users/test/repo/NewFile.swift",
                "content": "struct SyntheticValue {}",
            }),
        );
        let mut write_projection = RenderToolFieldProjection::from_message(&write, false).unwrap();
        assert_eq!(write_projection.kind, RenderToolKind::FileWrite);
        assert_file_change_slots(&write_projection, &["input", "file_path"]);
        assert_eq!(
            serde_json::to_value(write_projection.diff.as_ref().unwrap()).unwrap(),
            json!({
                "source": "tool_use",
                "segments": [{
                    "pair": {
                        "old": null,
                        "new": {"root": "content", "path": ["input", "content"]},
                    }
                }]
            })
        );
        write_projection.absorb_result(
            RenderToolFieldProjection::from_message(
                &result("Write", json!({"result": "Created synthetic file"})),
                true,
            )
            .unwrap(),
        );
        assert_file_change_slots(&write_projection, &["input", "file_path"]);

        let multi_edit = call(
            "MultiEdit",
            json!({
                "file_path": "/Users/test/repo/Multi.swift",
                "edits": [
                    {"old_string": "first old", "new_string": "first new"},
                    {"old_string": "second old", "new_string": "second new"},
                ],
            }),
        );
        let multi_projection = RenderToolFieldProjection::from_message(&multi_edit, false).unwrap();
        assert_file_change_slots(&multi_projection, &["input", "file_path"]);
        assert_eq!(
            multi_projection.diff.as_ref().unwrap().segments(),
            &[
                RenderToolDiffSegment::Pair(RenderToolDiffPair::old_new(
                    RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec![
                            "input".into(),
                            "edits".into(),
                            "0".into(),
                            "old_string".into()
                        ],
                    },
                    RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec![
                            "input".into(),
                            "edits".into(),
                            "0".into(),
                            "new_string".into()
                        ],
                    },
                )),
                RenderToolDiffSegment::Pair(RenderToolDiffPair::old_new(
                    RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec![
                            "input".into(),
                            "edits".into(),
                            "1".into(),
                            "old_string".into()
                        ],
                    },
                    RenderToolValueSelector {
                        root: RenderToolFieldRoot::Content,
                        path: vec![
                            "input".into(),
                            "edits".into(),
                            "1".into(),
                            "new_string".into()
                        ],
                    },
                )),
            ]
        );

        let notebook = call(
            "NotebookEdit",
            json!({
                "notebook_path": "/Users/test/repo/Sample.ipynb",
                "new_source": "print('synthetic')",
            }),
        );
        let notebook_projection =
            RenderToolFieldProjection::from_message(&notebook, false).unwrap();
        assert_eq!(notebook_projection.kind, RenderToolKind::FileEdit);
        assert_file_change_slots(&notebook_projection, &["input", "notebook_path"]);
        assert_eq!(
            notebook_projection.diff.as_ref().unwrap().segments(),
            &[RenderToolDiffSegment::Pair(RenderToolDiffPair::insert(
                RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["input".into(), "new_source".into()],
                }
            ))]
        );

        for forbidden in [
            "let oldValue = 1",
            "let newValue = 2",
            "struct SyntheticValue {}",
            "first old",
            "print('synthetic')",
        ] {
            assert!(
                !serde_json::to_string(&[
                    &edit_projection,
                    &write_projection,
                    &multi_projection,
                    &notebook_projection,
                ])
                .unwrap()
                .contains(forbidden)
            );
        }
    }

    #[test]
    fn empty_structured_file_changes_keep_exclusive_slots_and_adopt_result_recipe() {
        let cases = [
            (
                "Write",
                json!({"file_path": "/Users/test/Empty.txt", "content": ""}),
            ),
            (
                "Edit",
                json!({
                    "file_path": "/Users/test/Empty.txt",
                    "old_string": "",
                    "new_string": "",
                }),
            ),
            (
                "MultiEdit",
                json!({"file_path": "/Users/test/Empty.txt", "edits": []}),
            ),
            (
                "NotebookEdit",
                json!({"notebook_path": "/Users/test/Empty.ipynb", "new_source": ""}),
            ),
        ];

        for (tool_name, input) in cases {
            let call_message = call(tool_name, input);
            let analysis =
                analyze_special_shapes(&call_message, classify_tool(Some(tool_name)), false);
            assert_eq!(analysis, Some(SpecialShapeAnalysis::ComposableEmpty));

            let mut projection =
                RenderToolFieldProjection::from_message(&call_message, false).unwrap();
            let path_key = if tool_name == "NotebookEdit" {
                "notebook_path"
            } else {
                "file_path"
            };
            assert_file_change_slots(&projection, &["input", path_key]);
            assert_eq!(projection.diff, None);

            projection.absorb_result(
                RenderToolFieldProjection::from_message(
                    &result(tool_name, json!({"diff": "+result-side"})),
                    true,
                )
                .unwrap(),
            );
            assert_file_change_slots(&projection, &["input", path_key]);
            assert_eq!(
                projection.diff.as_ref().map(RenderToolDiffRecipe::source),
                Some(RenderToolDiffSource::ToolResult)
            );
        }

        let whitespace = call(
            "Write",
            json!({"file_path": "/Users/test/Whitespace.txt", "content": " \n"}),
        );
        assert!(matches!(
            analyze_special_shapes(&whitespace, RenderToolKind::FileWrite, false),
            Some(SpecialShapeAnalysis::Composable { .. })
        ));
        assert!(
            RenderToolFieldProjection::from_message(&whitespace, false)
                .unwrap()
                .diff
                .is_some()
        );
    }

    #[test]
    fn codex_file_change_enumerates_unified_segments_and_merge_is_call_wins() {
        let call_message = direct_call(
            "fileChange",
            json!({
                "type": "fileChange",
                "changes": [
                    {"path": "/Users/test/A.txt", "diff": "-old-a\n+new-a"},
                    {"path": "/Users/test/B.txt", "diff": "+new-b"},
                ],
            }),
        );
        let result_message = result(
            "fileChange",
            json!({
                "changes": [{"path": "/Users/test/A.txt", "diff": "+different-result"}],
                "status": "completed",
            }),
        );
        let mut projection = RenderToolFieldProjection::from_message(&call_message, false).unwrap();
        assert_eq!(projection.call, None);
        assert_eq!(
            projection.diff.as_ref().unwrap().segments(),
            &[
                RenderToolDiffSegment::Unified(RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["changes".into(), "0".into(), "diff".into()],
                }),
                RenderToolDiffSegment::Unified(RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["changes".into(), "1".into(), "diff".into()],
                }),
            ]
        );
        projection
            .absorb_result(RenderToolFieldProjection::from_message(&result_message, true).unwrap());
        assert_eq!(
            projection.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolUse)
        );
        assert_eq!(projection.diff.as_ref().unwrap().segments().len(), 2);

        let result_only = RenderToolFieldProjection::from_message(&result_message, true).unwrap();
        assert_eq!(
            result_only.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolResult)
        );
        assert_eq!(result_only.diff.as_ref().unwrap().segments().len(), 1);
    }

    #[test]
    fn generic_diff_is_orthogonal_to_precisely_selected_scalar_slots() {
        let diff_only = direct_call("custom_tool", json!({"diff": "+added"}));
        let projection = RenderToolFieldProjection::from_message(&diff_only, false).unwrap();
        assert_eq!(projection.call, None);
        assert!(projection.diff.is_some());

        let precise_call = direct_call(
            "custom_tool",
            json!({"title": "Synthetic call", "foo": 1, "bar": 2, "diff": "+added"}),
        );
        let projection = RenderToolFieldProjection::from_message(&precise_call, false).unwrap();
        assert_eq!(selector_path(projection.call.as_ref()), Some(vec!["title"]));
        assert_ne!(
            projection.call.as_ref().unwrap().format,
            RenderToolFieldFormat::Json
        );
        assert!(projection.diff.is_some());

        let no_precise_call =
            direct_call("custom_tool", json!({"foo": 1, "bar": 2, "diff": "+added"}));
        let projection = RenderToolFieldProjection::from_message(&no_precise_call, false).unwrap();
        assert_eq!(projection.call, None);
        assert!(projection.diff.is_some());

        let no_precise_result =
            result("custom_tool", json!({"foo": 1, "bar": 2, "diff": "+added"}));
        let projection = RenderToolFieldProjection::from_message(&no_precise_result, true).unwrap();
        assert_eq!(projection.result, None);
        assert!(projection.diff.is_some());

        let output_and_diff = result(
            "custom_tool",
            json!({"output": "Synthetic output", "diff": "+added"}),
        );
        let projection = RenderToolFieldProjection::from_message(&output_and_diff, true).unwrap();
        assert_eq!(
            selector_path(projection.result.as_ref()),
            Some(vec!["output"])
        );
        assert_eq!(
            projection.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolResult)
        );

        let nested_diff = direct_call("custom_tool", json!({"diff": {"diff": "+nested"}}));
        let projection = RenderToolFieldProjection::from_message(&nested_diff, false).unwrap();
        assert_eq!(
            projection.diff.as_ref().unwrap().segments(),
            &[RenderToolDiffSegment::Unified(RenderToolValueSelector {
                root: RenderToolFieldRoot::Content,
                path: vec!["diff".into(), "diff".into()],
            })]
        );

        let unsupported = direct_call("custom_tool", json!({"diff": {"note": "still reachable"}}));
        let projection = RenderToolFieldProjection::from_message(&unsupported, false).unwrap();
        assert_eq!(projection.diff, None);
        assert_eq!(selector_path(projection.call.as_ref()), Some(vec![]));
        assert_eq!(
            projection.call.as_ref().unwrap().format,
            RenderToolFieldFormat::Json
        );
    }

    #[test]
    fn analyzer_verdict_is_pass_atomic_on_call_and_result_sides() {
        for is_result in [false, true] {
            let message = if is_result {
                result("custom_tool", json!({"foo": 1, "bar": 2, "diff": ""}))
            } else {
                direct_call("custom_tool", json!({"foo": 1, "bar": 2, "diff": ""}))
            };
            assert_eq!(
                analyze_special_shapes(&message, RenderToolKind::Generic, is_result),
                Some(SpecialShapeAnalysis::ComposableEmpty)
            );
            let projection = RenderToolFieldProjection::from_message(&message, is_result).unwrap();
            let scalar = if is_result {
                projection.result.as_ref()
            } else {
                projection.call.as_ref()
            };
            assert_eq!(selector_path(scalar), Some(vec![]));
            assert_eq!(scalar.unwrap().format, RenderToolFieldFormat::Json);

            let message = if is_result {
                result("custom_tool", json!({"foo": 1, "bar": 2, "changes": []}))
            } else {
                direct_call("custom_tool", json!({"foo": 1, "bar": 2, "changes": []}))
            };
            assert_eq!(
                analyze_special_shapes(&message, RenderToolKind::Generic, is_result),
                Some(SpecialShapeAnalysis::ComposableEmpty)
            );

            let message = if is_result {
                result(
                    "custom_tool",
                    json!({"foo": 1, "bar": 2, "changes": [{"diff": "+x"}, {"diff": ""}]}),
                )
            } else {
                direct_call(
                    "custom_tool",
                    json!({"foo": 1, "bar": 2, "changes": [{"diff": "+x"}, {"diff": ""}]}),
                )
            };
            let analysis = analyze_special_shapes(&message, RenderToolKind::Generic, is_result);
            assert!(matches!(
                analysis,
                Some(SpecialShapeAnalysis::Composable { ref segments, .. }) if segments.len() == 1
            ));
            let projection = RenderToolFieldProjection::from_message(&message, is_result).unwrap();
            assert!(projection.diff.is_some());
            assert_eq!(
                if is_result {
                    projection.result
                } else {
                    projection.call
                },
                None
            );

            for content in [
                json!({
                    "foo": 1,
                    "bar": 2,
                    "changes": [{"diff": "+x"}, {"note": "keep"}],
                }),
                json!({"changes": "+x", "diff": {"note": "keep"}}),
            ] {
                let message = if is_result {
                    result("custom_tool", content)
                } else {
                    direct_call("custom_tool", content)
                };
                assert_eq!(
                    analyze_special_shapes(&message, RenderToolKind::Generic, is_result),
                    Some(SpecialShapeAnalysis::Unsupported)
                );
                let projection =
                    RenderToolFieldProjection::from_message(&message, is_result).unwrap();
                assert_eq!(projection.diff, None);
                let scalar = if is_result {
                    projection.result.as_ref()
                } else {
                    projection.call.as_ref()
                };
                assert_eq!(selector_path(scalar), Some(vec![]));
                assert_eq!(scalar.unwrap().format, RenderToolFieldFormat::Json);
            }
        }
    }

    #[test]
    fn pair_grammar_prunes_empty_edits_and_rejects_a_mixed_invalid_array() {
        let valid = call(
            "MultiEdit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "edits": [{"old_string": "old", "new_string": "new"}],
            }),
        );
        assert!(matches!(
            analyze_special_shapes(&valid, RenderToolKind::FileEdit, false),
            Some(SpecialShapeAnalysis::Composable { ref segments, .. }) if segments.len() == 1
        ));

        let empty = call(
            "MultiEdit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "edits": [{"old_string": "", "new_string": ""}],
            }),
        );
        assert_eq!(
            analyze_special_shapes(&empty, RenderToolKind::FileEdit, false),
            Some(SpecialShapeAnalysis::ComposableEmpty)
        );

        let invalid = call(
            "MultiEdit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "edits": [{"old_string": "old", "new_string": "new"}, 42],
            }),
        );
        assert_eq!(
            analyze_special_shapes(&invalid, RenderToolKind::FileEdit, false),
            Some(SpecialShapeAnalysis::Unsupported)
        );
        let projection = RenderToolFieldProjection::from_message(&invalid, false).unwrap();
        assert_eq!(projection.diff, None);
        assert_eq!(projection.summary, None);
        assert_eq!(
            selector_path(projection.call.as_ref()),
            Some(vec!["input", "file_path"])
        );
    }

    #[test]
    fn file_slot_exclusivity_is_gated_only_by_the_atomic_verdict() {
        let unsupported = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "diff": {"note": "keep"},
            }),
        );
        let projection = RenderToolFieldProjection::from_message(&unsupported, false).unwrap();
        assert_eq!(projection.diff, None);
        assert_eq!(projection.summary, None);
        assert_eq!(
            selector_path(projection.call.as_ref()),
            Some(vec!["input", "file_path"])
        );

        let valid_pair_with_unsupported_sibling = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "old_string": "a",
                "new_string": "b",
                "diff": {"note": "keep"},
            }),
        );
        assert_eq!(
            analyze_special_shapes(
                &valid_pair_with_unsupported_sibling,
                RenderToolKind::FileEdit,
                false,
            ),
            Some(SpecialShapeAnalysis::Unsupported)
        );
        let projection =
            RenderToolFieldProjection::from_message(&valid_pair_with_unsupported_sibling, false)
                .unwrap();
        assert_eq!(projection.diff, None);
        assert_eq!(projection.summary, None);
        assert_eq!(
            selector_path(projection.call.as_ref()),
            Some(vec!["input", "file_path"])
        );

        let legal_zero = call(
            "Edit",
            json!({"file_path": "/Users/test/Synthetic.txt", "diff": ""}),
        );
        let projection = RenderToolFieldProjection::from_message(&legal_zero, false).unwrap();
        assert_file_change_slots(&projection, &["input", "file_path"]);
        assert_eq!(projection.diff, None);
    }

    #[test]
    fn segment_ordering_scans_the_whole_pass_before_choosing_the_first_group() {
        let both_pre_rendered = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "changes": "+a",
                "diff": "+b",
            }),
        );
        let projection =
            RenderToolFieldProjection::from_message(&both_pre_rendered, false).unwrap();
        assert_eq!(
            projection.diff.as_ref().unwrap().segments(),
            &[
                RenderToolDiffSegment::Unified(RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["input".into(), "changes".into()],
                }),
                RenderToolDiffSegment::Unified(RenderToolValueSelector {
                    root: RenderToolFieldRoot::Content,
                    path: vec!["input".into(), "diff".into()],
                }),
            ]
        );

        let empty_pre = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "diff": "",
                "old_string": "a",
                "new_string": "b",
            }),
        );
        let projection = RenderToolFieldProjection::from_message(&empty_pre, false).unwrap();
        assert!(matches!(
            projection.diff.as_ref().unwrap().segments(),
            [RenderToolDiffSegment::Pair(_)]
        ));

        let pre_wins = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "diff": "+pre",
                "old_string": "a",
                "new_string": "b",
            }),
        );
        let projection = RenderToolFieldProjection::from_message(&pre_wins, false).unwrap();
        assert!(matches!(
            projection.diff.as_ref().unwrap().segments(),
            [RenderToolDiffSegment::Unified(_)]
        ));

        let later_invalid = call(
            "Edit",
            json!({
                "file_path": "/Users/test/Synthetic.txt",
                "diff": "+pre",
                "old_string": 42,
                "new_string": "b",
            }),
        );
        assert_eq!(
            analyze_special_shapes(&later_invalid, RenderToolKind::FileEdit, false),
            Some(SpecialShapeAnalysis::Unsupported)
        );
        let projection = RenderToolFieldProjection::from_message(&later_invalid, false).unwrap();
        assert_eq!(projection.diff, None);
        assert_eq!(projection.summary, None);
        assert_eq!(
            selector_path(projection.call.as_ref()),
            Some(vec!["input", "file_path"])
        );
    }

    #[test]
    fn no_candidate_file_operations_keep_their_existing_scalar_slots() {
        for tool_name in ["DeleteFile", "RenameFile", "MoveFile"] {
            let message = call(tool_name, json!({"file_path": "/Users/test/Synthetic.txt"}));
            assert_eq!(
                analyze_special_shapes(&message, RenderToolKind::FileEdit, false),
                None
            );
            let projection = RenderToolFieldProjection::from_message(&message, false).unwrap();
            assert_eq!(projection.summary, None);
            assert_eq!(
                selector_path(projection.call.as_ref()),
                Some(vec!["input", "file_path"])
            );
            assert_eq!(projection.diff, None);
        }

        let apply_patch = call(
            "ApplyPatch",
            json!({"file_path": "/Users/test/Synthetic.txt", "patch": "synthetic patch"}),
        );
        assert_eq!(
            analyze_special_shapes(&apply_patch, RenderToolKind::FileEdit, false),
            None
        );
        let projection = RenderToolFieldProjection::from_message(&apply_patch, false).unwrap();
        assert_eq!(
            selector_path(projection.call.as_ref()),
            Some(vec!["input", "file_path"])
        );
    }

    #[test]
    fn recipe_merge_truth_table_is_call_wins_with_empty_analysis_as_none() {
        let call_recipe = RenderToolFieldProjection::from_message(
            &direct_call("custom_tool", json!({"diff": "+call"})),
            false,
        );
        let call_empty = RenderToolFieldProjection::from_message(
            &direct_call("custom_tool", json!({"diff": ""})),
            false,
        );
        let result_recipe = RenderToolFieldProjection::from_message(
            &result("custom_tool", json!({"diff": "+result"})),
            true,
        );
        let result_empty = RenderToolFieldProjection::from_message(
            &result("custom_tool", json!({"diff": ""})),
            true,
        );

        let none_none = merge_tool_result_projection(call_empty.clone(), result_empty.clone())
            .expect("metadata still keeps a projection");
        assert_eq!(none_none.diff, None);

        let some_none = merge_tool_result_projection(call_recipe.clone(), result_empty).unwrap();
        assert_eq!(
            some_none.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolUse)
        );

        let none_some = merge_tool_result_projection(call_empty, result_recipe.clone()).unwrap();
        assert_eq!(
            none_some.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolResult)
        );

        let some_some = merge_tool_result_projection(call_recipe, result_recipe).unwrap();
        assert_eq!(
            some_some.diff.as_ref().map(RenderToolDiffRecipe::source),
            Some(RenderToolDiffSource::ToolUse)
        );

        assert_eq!(merge_tool_result_projection(None, None), None);
    }

    #[test]
    fn illegal_diff_recipes_fail_decode_and_checked_api_serializes_only_legal_shapes() {
        for illegal in [
            json!({"source": "tool_use", "segments": []}),
            json!({
                "source": "tool_use",
                "segments": [{"pair": {"old": null, "new": null}}],
            }),
            json!({
                "source": "tool_use",
                "segments": [{"pair": {}}],
            }),
        ] {
            assert!(serde_json::from_value::<RenderToolDiffRecipe>(illegal).is_err());
        }

        assert_eq!(
            RenderToolDiffRecipe::new(RenderToolDiffSource::ToolUse, Vec::new()),
            None
        );
        let legal = RenderToolDiffRecipe::new(
            RenderToolDiffSource::ToolUse,
            vec![RenderToolDiffSegment::Pair(RenderToolDiffPair::insert(
                RenderToolValueSelector {
                    root: RenderToolFieldRoot::Input,
                    path: vec!["content".into()],
                },
            ))],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&legal).unwrap(),
            json!({
                "source": "tool_use",
                "segments": [{
                    "pair": {
                        "old": null,
                        "new": {"root": "input", "path": ["content"]},
                    }
                }]
            })
        );
    }

    #[test]
    fn scalar_selector_wire_bytes_are_unchanged_after_location_factoring() {
        for wire in [
            r#"{"root":"content","path":["input","command"],"format":"code","label":"command"}"#,
            r#"{"root":"result","format":"text","label":"result"}"#,
        ] {
            let selector = serde_json::from_str::<RenderToolFieldSelector>(wire).unwrap();
            assert_eq!(serde_json::to_string(&selector).unwrap(), wire);
        }
    }

    #[test]
    fn retired_scalar_diff_tokens_have_no_producer_or_consumer() {
        let source = include_str!("tool_field_projection.rs");
        for retired in [
            ["RenderToolFieldFormat::", "Diff"].concat(),
            ["RenderToolFieldLabel::", "Diff"].concat(),
        ] {
            assert!(!source.contains(&retired));
        }
    }
}
