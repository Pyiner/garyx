import { ToggleGroup, ToggleGroupItem } from 'garyx-desktop'
import {
  LayoutGrid,
  List,
  Rows3,
  Bold,
  Italic,
  Underline,
  CircleDot,
  CheckCircle2,
  Archive,
} from 'lucide-react'

export function ViewSwitcher() {
  return (
    <ToggleGroup type="single" variant="outline" defaultValue="list">
      <ToggleGroupItem value="list" aria-label="List view">
        <List />
        List
      </ToggleGroupItem>
      <ToggleGroupItem value="board" aria-label="Board view">
        <LayoutGrid />
        Board
      </ToggleGroupItem>
      <ToggleGroupItem value="compact" aria-label="Compact view">
        <Rows3 />
        Compact
      </ToggleGroupItem>
    </ToggleGroup>
  )
}

export function StatusFilter() {
  return (
    <ToggleGroup type="multiple" variant="outline" spacing={1} defaultValue={['active', 'review']}>
      <ToggleGroupItem value="active">
        <CircleDot />
        Active
      </ToggleGroupItem>
      <ToggleGroupItem value="review">
        <CheckCircle2 />
        In review
      </ToggleGroupItem>
      <ToggleGroupItem value="archived">
        <Archive />
        Archived
      </ToggleGroupItem>
    </ToggleGroup>
  )
}

export function FormatToolbar() {
  return (
    <ToggleGroup type="multiple" variant="outline" defaultValue={['bold']}>
      <ToggleGroupItem value="bold" aria-label="Bold">
        <Bold />
      </ToggleGroupItem>
      <ToggleGroupItem value="italic" aria-label="Italic">
        <Italic />
      </ToggleGroupItem>
      <ToggleGroupItem value="underline" aria-label="Underline">
        <Underline />
      </ToggleGroupItem>
    </ToggleGroup>
  )
}

export function Spaced() {
  return (
    <ToggleGroup type="single" variant="outline" spacing={2} defaultValue="board">
      <ToggleGroupItem value="list">List</ToggleGroupItem>
      <ToggleGroupItem value="board">Board</ToggleGroupItem>
      <ToggleGroupItem value="compact">Compact</ToggleGroupItem>
    </ToggleGroup>
  )
}
