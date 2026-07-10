// Design-sync barrel entry. Re-exports the Garyx desktop UI component library
// (the shadcn-style primitives under src/renderer/src/components/ui) so esbuild
// can bundle them into window.GaryxUI. `export *` exposes every compound part
// (CardHeader, DialogContent, SelectItem, …) on the global, not just the roots.
//
// This file is a sync input, not app code — it is read only by the design-sync
// converter. See .design-sync/NOTES.md.
export * from '@/components/ui/avatar';
export * from '@/components/ui/badge';
export * from '@/components/ui/button';
export * from '@/components/ui/card';
export * from '@/components/ui/checkbox';
export * from '@/components/ui/dialog';
export * from '@/components/ui/dropdown-menu';
export * from '@/components/ui/field';
export * from '@/components/ui/floating-action-menu';
export * from '@/components/ui/input';
export * from '@/components/ui/label';
export * from '@/components/ui/popover';
export * from '@/components/ui/select';
export * from '@/components/ui/separator';
export * from '@/components/ui/switch';
export * from '@/components/ui/table';
export * from '@/components/ui/textarea';
export * from '@/components/ui/toggle';
export * from '@/components/ui/toggle-group';

// app-shell composites (identity, banners, panels)
export * from '@/app-shell/components/ProviderAgentIcon';
export * from '@/app-shell/components/AgentAvatar';
export * from '@/app-shell/components/AgentOptionAvatar';
export * from '@/app-shell/components/RateLimitBanner';
export * from '@/app-shell/components/UpdatePill';
export * from '@/app-shell/components/RendererPerformancePanel';
export * from '@/app-shell/components/ThreadLogPanel';
export * from '@/app-shell/components/TaskForestConsole';
