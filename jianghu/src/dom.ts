export function requiredElement<E extends Element>(root: ParentNode, selector: string, kind: new () => E): E {
  const element = root.querySelector(selector);
  if (!(element instanceof kind)) throw new Error(`Expected ${kind.name} for "${selector}".`);
  return element;
}
