// Self-referential module (imports itself - edge case)
// This is technically valid TypeScript for type-only self-references

export interface TreeNode {
  value: string;
  children: TreeNode[]; // Self-referential type
}

export function createTree(value: string, children: TreeNode[] = []): TreeNode {
  return { value, children };
}

// Function that calls itself (recursive, not a circular import but worth testing)
export function flattenTree(node: TreeNode): string[] {
  return [node.value, ...node.children.flatMap(flattenTree)];
}
