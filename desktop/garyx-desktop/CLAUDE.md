# Garyx — Design Decisions

## Global focus outline: none

`*:focus { outline: none }` is an intentional design choice. Do NOT "fix" it to `:focus-visible` or add focus ring styles. This is by design.

## Design Context

### Users
Mixed audience — both technical users (developers using AI agents for coding, debugging, and automation) and non-technical users (knowledge workers using agents for writing, research, and daily work). Team managers who orchestrate multiple agents and agent teams. Users interact with the app on macOS as a desktop productivity tool.

### Brand Personality
**Professional, refined, restrained** — Like Linear or Notion. Clean, efficient, no unnecessary decoration. Every element earns its place. The interface should feel like a precision instrument, not a toy.

### Aesthetic Direction
- **Visual tone**: Consistent with the existing macOS app design. Warm neutral palette (slightly warm whites: `#fafaf9`, `#f4f4f2`), SF Pro system font stack for native macOS feel.
- **References**: Linear (information density, restraint), Things 3 (warm-neutral palette, calm green accent), macOS native apps (system integration, familiar patterns).
- **Anti-references**: Overly playful AI tools, dark mode with neon accents, gradient text, glassmorphism for decoration.
- **Theme**: Light mode only. Warm neutrals with green accent (`#00a240` / `#2e7d32`) for primary actions. Borders are soft (`#eeeeee`), shadows are subtle.
- **Key constraint**: Must feel like it belongs on macOS — use system fonts, native-feeling spacing, and avoid web-app aesthetics that break the desktop illusion.

### Design Principles
1. **Restraint over decoration** — Remove before adding. If an element doesn't help the user complete a task, it doesn't belong.
2. **Native coherence** — The app should feel like a natural extension of macOS. System fonts, familiar interaction patterns, no jarring web-isms.
3. **Information density done right** — Show what matters, hide what doesn't. Progressive disclosure over overwhelming upfront.
4. **Consistency across views** — Every page should feel like it belongs to the same app. Shared tokens, spacing rhythm, interaction patterns.
5. **Quiet confidence** — The design should communicate competence without shouting. Subtle shadows, muted colors, precise typography.
