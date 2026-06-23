import { Button } from 'garyx-desktop';
import { Plus, Trash2, RefreshCw } from 'lucide-react';

const row: React.CSSProperties = { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 12, padding: 16 };

export const Variants = () => (
  <div style={row}>
    <Button>New thread</Button>
    <Button variant="secondary">Secondary</Button>
    <Button variant="outline">Outline</Button>
    <Button variant="ghost">Ghost</Button>
    <Button variant="link">Link</Button>
    <Button variant="destructive">Delete</Button>
  </div>
);

export const Sizes = () => (
  <div style={row}>
    <Button size="xs">Extra small</Button>
    <Button size="sm">Small</Button>
    <Button size="default">Default</Button>
    <Button size="lg">Large</Button>
  </div>
);

export const WithIcons = () => (
  <div style={row}>
    <Button><Plus /> New agent</Button>
    <Button variant="outline"><RefreshCw /> Restart gateway</Button>
    <Button variant="destructive"><Trash2 /> Remove bot</Button>
    <Button size="icon" variant="outline"><Plus /></Button>
  </div>
);

export const States = () => (
  <div style={row}>
    <Button>Enabled</Button>
    <Button disabled>Disabled</Button>
    <Button variant="outline" disabled>Disabled outline</Button>
  </div>
);
