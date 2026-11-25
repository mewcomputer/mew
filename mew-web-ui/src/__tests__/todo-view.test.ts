import { describe, expect, it } from "vitest";
import { summarizeTodos } from "../lib/todo-view";
import type { TodoItem } from "../stores/session";

const todo = (id: number, status: TodoItem["status"]): TodoItem => ({ id, content: `step ${id}`, status, dependsOn: [] });

describe("summarizeTodos", () => {
  it("puts open work first and leaves finished items for the remaining rows", () => {
    const summary = summarizeTodos([
      todo(1, "done"), todo(2, "done"), todo(3, "in_progress"), todo(4, "pending"),
    ]);
    expect(summary.rows.map((row) => row.id)).toEqual([3, 4, 2, 1]);
    expect(summary.open).toBe(2);
    expect(summary.done).toBe(2);
    expect(summary.hidden).toBe(0);
  });

  it("keeps the newest finished items and hides the older plan", () => {
    const summary = summarizeTodos([
      todo(1, "done"), todo(2, "done"), todo(3, "done"), todo(4, "done"), todo(5, "done"),
    ], 3);
    expect(summary.rows.map((row) => row.id)).toEqual([5, 4, 3]);
    expect(summary.hidden).toBe(2);
    expect(summary.open).toBe(0);
  });

  it("never drops open work for finished work", () => {
    const summary = summarizeTodos([
      todo(1, "done"), todo(2, "pending"), todo(3, "blocked"), todo(4, "in_progress"),
    ], 3);
    expect(summary.rows.map((row) => row.id)).toEqual([2, 3, 4]);
    expect(summary.hidden).toBe(1);
  });

  it("reports an empty plan without inventing rows", () => {
    const summary = summarizeTodos([]);
    expect(summary.rows).toEqual([]);
    expect(summary.hidden).toBe(0);
  });
});
