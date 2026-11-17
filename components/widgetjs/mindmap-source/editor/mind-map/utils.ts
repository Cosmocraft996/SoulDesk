import type { MindNode } from '../types/MindNode'

export function getMindNodeData(node: any): MindNode {
  const { ...restData } = node.data || {}
  return {
    ...restData,
    id: node.id,
    checked: node.checked ?? restData.checked,
    collapsed: node.collapsed,
    children: node.children?.map(getMindNodeData) || [],
  }
}

export function findNodeInTree(root: any, id: string): any {
  if (!root) { return null }
  if (root.id === id) { return root }
  if (root.children) {
    for (const child of root.children) {
      const found = findNodeInTree(child, id)
      if (found) { return found }
    }
  }
  return null
}

export function findNodeWithParent(root: any, id: string, parent: any = null): { node: any, parent: any, index: number } | null {
  if (!root) { return null }
  if (root.id === id) {
    return { node: root, parent, index: -1 }
  }
  if (root.children) {
    for (let i = 0; i < root.children.length; i++) {
      const child = root.children[i]
      if (child.id === id) {
        return { node: child, parent: root, index: i }
      }
      const found = findNodeWithParent(child, id, root)
      if (found) { return found }
    }
  }
  return null
}

// 数据转换：为每个节点添加唯一 ID
export function transformData(data: any) {
  const result: any = {
    id: data.id || Math.random().toString(36).slice(2),
    data: { ...data }, // keep original data
    children: [],
    collapsed: data.collapsed ?? false, // default collapsed state
    checked: data.checked ?? false,
  }
  if (data.children) {
    result.children = data.children.map((child: any) => transformData(child))
  }
  return result
}

// Filter collapsed nodes for layout
export function getVisibleData(node: any): any {
  const visibleNode = {
    ...node,
    children: node.collapsed ? [] : (node.children || []).map(getVisibleData),
  }
  return visibleNode
}

// 获取所有后代节点 ID
export function getAllDescendants(root: any, id: string): string[] {
  const descendants: string[] = []
  const node = findNodeInTree(root, id)
  if (node && node.children) {
    const traverse = (n: any) => {
      if (n.children) {
        n.children.forEach((child: any) => {
          descendants.push(child.id)
          traverse(child)
        })
      }
    }
    traverse(node)
  }
  return descendants
}
