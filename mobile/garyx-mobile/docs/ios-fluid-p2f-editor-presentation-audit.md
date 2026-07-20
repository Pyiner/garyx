# P2-F compact editor presentation audit

This audit applies one product rule: a bounded single-record form can preserve
its parent context in a detent sheet; long-form authoring, hierarchical
browsing, connection/authentication flows, and dynamic multi-section forms
retain a full-screen workspace. No information architecture or data contract
changes are part of this pass.

## Converted to sheets

| Surface | Presentation | Rationale |
|---|---|---|
| Skills · Edit Skill Info | `.medium` + `.large` | Two metadata fields; the medium detent preserves list context and the large detent accommodates the keyboard. |
| Commands · Add Command | `.large` | One three-field record. Long content already delegates to the dedicated focused text editor. |
| Commands · Edit Command | `.large` | Same bounded record shape as create; nested focused text editing remains available. |
| Settings · Edit Gateway | `.large` | One saved profile with four related fields; large height keeps secure input and headers usable with the keyboard. |

The shared Commands surfaces are used both from the Commands panel and the
Settings Commands tab, so both entry points now use the same sheet contract.

## Retained full-screen presentations

| Surface | Rationale |
|---|---|
| Skills · New Skill | Includes long Body authoring and can enter the immersive focused editor; it is a document-creation workspace, not compact metadata. |
| Skills · Skill Detail | Combines metadata, a hierarchical file browser, and document preview; reducing it to a detent would remove useful reading space. |
| Focused text editor | Intentionally isolates long prompt/body editing from the surrounding form and keyboard chrome. |
| Settings · Add Gateway | Save-and-connect workflow with live connection state, validation, retry, and gateway switching; it is a task flow rather than a profile edit. |
| Settings · Add/Edit Bot | Schema-driven credentials plus channel, agent, workspace, and dynamic authentication fields can span many sections. |
| Settings · Add/Edit MCP Server | Multiple Command and HTTP configuration groups, including environment variables and headers, need a full-height form. |
| Settings · Provider Defaults | Authoritative hydration, quota, authentication, runtime sections, and nested login make this a stateful management workspace. |

Native option-selection sheets in Agents already use detents and system sheet
materialization, so this audit does not replace them with custom transitions.
