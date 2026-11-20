import { parser } from "@lezer/python";

/** Format indentation for display only, retaining the original source separately. */
export function formatPythonForDisplay(source: string): string {
  const strings: Array<[number, number]> = [];
  let invalid = false;
  parser.parse(source).iterate({ enter(node) {
    if (node.type.isError) invalid = true;
    if (node.name === "String" || node.name === "FormatString") strings.push([node.from, node.to]);
  } });
  if (invalid) return source;

  const lines = source.split("\n");
  let offset = 0;
  const indents = lines.map((line) => {
    const inString = strings.some(([start, end]) => start < offset && offset < end);
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
