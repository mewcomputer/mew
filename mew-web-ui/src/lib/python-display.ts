import { parser } from "@lezer/python";

export const MAX_PYTHON_DISPLAY_LENGTH = 16 * 1024;

/** Format indentation for display only, retaining the original source separately. */
export function formatPythonForDisplay(source: string): string {
  if (source.length > MAX_PYTHON_DISPLAY_LENGTH) return source;
  const strings: Array<[number, number]> = [];
  let invalid = false;
  parser.parse(source).iterate({ enter(node) {
    if (node.type.isError) invalid = true;
    if (node.name === "String" || node.name === "FormatString") strings.push([node.from, node.to]);
  } });
  if (invalid) return source;

  const lines = source.split("\n");
  let offset = 0;
  let stringIndex = 0;
  const indents = lines.map((line) => {
    while (stringIndex < strings.length && strings[stringIndex]![1] <= offset) stringIndex++;
    const range = strings[stringIndex];
    const inString = range !== undefined && range[0] < offset && offset < range[1];
    offset += line.length + 1;
    if (inString || !line.trim()) return null;
    const indent = line.match(/^[ \t]*/)?.[0] ?? "";
    return indent.includes("\t") ? -1 : indent.length;
  });
  if (indents.includes(-1)) return source;
  const nonzero = indents.filter((indent): indent is number => indent !== null && indent > 0);
  if (!nonzero.length) return source;
  const unit = nonzero.reduce((smallest, indent) => Math.min(smallest, indent));
  if (indents.some((indent) => indent !== null && indent % unit !== 0)) return source;
  return lines.map((line, index) => {
    const indent = indents[index];
    return indent ? " ".repeat(indent / unit * 2) + line.slice(indent) : line;
  }).join("\n");
}
