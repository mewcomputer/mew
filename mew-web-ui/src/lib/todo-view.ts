import type { TodoItem } from "../stores/session";

/** Rows worth showing in the overview, and the counts that describe the rest.
 *
 *  A long session accumulates every plan it ever wrote, so finished work would
 *  otherwise bury what is still open: open items come first in plan order, then
 *  the most recently created finished ones fill the remaining rows. */
export function summarizeTodos(todos: TodoItem[], limit = 5): {
  rows: TodoItem[];
  hidden: number;
  open: number;
  done: number;
} {
  const isOpen = (todo: TodoItem) => todo.status !== "done";
  const open = todos.filter(isOpen);
  const done = todos.filter((todo) => !isOpen(todo)).sort((a, b) => b.id - a.id);
  const rows = [...open, ...done].slice(0, limit);
  return {
    rows,
    hidden: todos.length - rows.length,
    open: open.length,
    done: done.length,
  };
}
