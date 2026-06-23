# Garyx Desktop UI — how to build with this library

A shadcn-style React component library for the Garyx macOS desktop app. Warm-neutral,
light-mode, system-font (SF Pro), green accent. Restrained and native-feeling — compose
the components; don't reskin them.

## Setup — no provider needed
Components are styled entirely by the shipped stylesheet (`styles.css`, which carries the
design tokens and the compiled component CSS). There is **no theme/provider wrapper to
mount** — import a component and render it; tokens resolve from `:root`. Just make sure
`styles.css` is loaded on the page.

```jsx
import { Button, Card, CardHeader, CardTitle, CardContent } from 'garyx-desktop';

<Card>
  <CardHeader><CardTitle>Claude Code</CardTitle></CardHeader>
  <CardContent>
    <Button>New thread</Button>
  </CardContent>
</Card>
```

## Styling idiom — compose components, use the CSS token variables for your own layout
The shipped stylesheet is **static**: it contains only the Tailwind utility classes the
library itself uses. So **do not assume arbitrary Tailwind classes resolve** — a class the
library never used (e.g. `bg-accent`, `bg-success`, `grid-cols-7`) will have no rule at
design time. Two safe ways to style:

1. **Lean on the components.** They carry their own styling — buttons, cards, inputs,
   dialogs, tables all render correct out of the box. Reach for a component before styling
   raw markup.
2. **For your own containers/layout, use inline styles backed by the design tokens.** The
   tokens are global CSS custom properties on `:root` — always available:

   | Token var | Meaning |
   |---|---|
   | `--background` / `--foreground` | page bg / primary text |
   | `--card` / `--card-foreground` | surface bg / text |
   | `--primary` / `--primary-foreground` | primary action (near-black) / on-primary |
   | `--secondary` / `--secondary-foreground` | subtle fill / text |
   | `--muted` / `--muted-foreground` | muted fill / secondary text |
   | `--accent` | row hover fill |
   | `--destructive` | error/danger red |
   | `--success` | green (`#00a240`, the brand accent) |
   | `--warning` | warning orange |
   | `--border` / `--input` / `--ring` | borders / input border / focus ring |
   | `--radius` | base corner radius |
   | `--font-sans` / `--font-mono` | SF Pro stack / SF Mono stack |

   ```jsx
   <div style={{
     background: 'var(--card)', color: 'var(--foreground)',
     border: '1px solid var(--border)', borderRadius: 'var(--radius)', padding: 16,
   }}>…</div>
   ```

   The utility classes the components DO ship (and you may reuse) include semantic-token
   ones like `bg-primary text-primary-foreground`, `bg-card`, `bg-secondary`, `bg-muted`,
   `text-foreground`, `text-muted-foreground`, `border-border`, `rounded-md`, `rounded-lg`,
   `font-mono`. When in doubt, inline styles + token vars are the reliable path.

## Where the truth lives
- `styles.css` (and the `_ds_bundle.css` it imports) — the tokens and compiled component
  styles. Read it to see exactly which classes exist.
- Each component's `<Name>.d.ts` is its prop contract; `<Name>.prompt.md` is its usage doc.
  Read those before composing a component you haven't used.

## Components available
Avatar, Badge, Button, Card, Checkbox, Dialog, DropdownMenu, Field, FloatingActionMenuContent,
Input, Label, Popover, Select, Separator, Switch, Table, Textarea, Toggle, ToggleGroup — plus
their compound parts (e.g. `CardHeader`/`CardContent`, `DialogContent`/`DialogFooter`,
`SelectTrigger`/`SelectItem`, `TableHeader`/`TableRow`/`TableCell`), all named exports of
`garyx-desktop`. `Button` and `Badge` take a `variant` prop (default / secondary / outline /
ghost / link / destructive); overlays (`Dialog`, `DropdownMenu`, `Popover`) follow the radix
trigger+content composition.

App-shell composites are also exported: `AgentAvatar`, `ProviderAgentIcon`, `AgentOptionAvatar`
(agent/team identity marks), `RateLimitBanner` (quota banner), and `RendererPerformancePanel`
(renderer health) — these carry app-specific styling and read simple data props.
