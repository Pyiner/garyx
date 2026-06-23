import {
  Button,
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from 'garyx-desktop';

const wrap: React.CSSProperties = { padding: 16, maxWidth: 420 };

export const Basic = () => (
  <div style={wrap}>
    <Card>
      <CardHeader>
        <CardTitle>Claude Code</CardTitle>
        <CardDescription>Anthropic · claude-opus-4-8</CardDescription>
      </CardHeader>
      <CardContent>
        <p style={{ margin: 0, fontSize: 14, lineHeight: 1.5, color: '#555' }}>
          A coding agent connected to your workspace. It can read, edit, and run
          code across the threads you route to it.
        </p>
      </CardContent>
      <CardFooter style={{ gap: 8 }}>
        <Button size="sm">Open thread</Button>
        <Button size="sm" variant="outline">Configure</Button>
      </CardFooter>
    </Card>
  </div>
);

export const WithAction = () => (
  <div style={wrap}>
    <Card>
      <CardHeader>
        <CardTitle>Telegram · main</CardTitle>
        <CardDescription>Connected · 3 active threads</CardDescription>
        <CardAction>
          <Button size="icon-sm" variant="ghost">⋯</Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        <p style={{ margin: 0, fontSize: 14, color: '#555' }}>
          Messages from this channel are throttled with 300ms edit coalescing.
        </p>
      </CardContent>
    </Card>
  </div>
);
