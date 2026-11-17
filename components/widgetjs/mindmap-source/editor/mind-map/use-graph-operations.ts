import type { Graph } from '@antv/x6'
import type { MindNode, MindNodeChangeType } from '../types/MindNode'
import { useState } from 'react'
import { toast } from 'sonner'
import { findNodeInTree, findNodeWithParent, getMindNodeData } from './utils'

interface UseGraphOperationsProps {
  graphRef: React.RefObject<Graph | null>
  treeDataRef: React.RefObject<any>
  renderRef: React.RefObject<(() => void) | null>
  selectedNodeIdRef: React.RefObject<string | null>
  onNodeChange?: (data: MindNode, type: MindNodeChangeType, changedNode?: MindNode) => void
}

/**
 * 核心图形操作 Hook
 * 负责处理节点的增删改查、移动等逻辑操作
 */
export function useGraphOperations({
  graphRef,
  treeDataRef,
  renderRef,
  selectedNodeIdRef,
  onNodeChange,
}: UseGraphOperationsProps) {
  // 重命名弹窗状态
  const [renameOpen, setRenameOpen] = useState(false)
  const [renameNodeId, setRenameNodeId] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')

  // 删除确认弹窗状态
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [deleteNodeId, setDeleteNodeId] = useState<string | null>(null)
  const [deleteNodeName, setDeleteNodeName] = useState('')

  /**
   * 打开重命名弹窗
   * @param nodeId 目标节点ID
   */
  const openRenameDialog = (nodeId: string) => {
    const sourceNode = findNodeInTree(treeDataRef.current, nodeId)
    if (!sourceNode) { return }
    setRenameNodeId(nodeId)
    setRenameValue(sourceNode.data?.name ?? '')
    setRenameOpen(true)
  }

  /**
   * 创建子节点
   * @param parentId 父节点ID
   * @param nodeValue 新节点名称
   */
  const applyCreateChild = (parentId: string, nodeValue: string) => {
    const pId = parentId
    if (!pId) { return }
    const nextName = nodeValue ? nodeValue.trim() : '新节点'
    if (!nextName) { return }

    const sourceNode = findNodeInTree(treeDataRef.current, pId)
    if (!sourceNode) { return }

    const newId = Math.random().toString(36).slice(2)
    const newChildData = {
      id: newId,
      name: nextName,
      children: [],
    }
    const newChildNode = {
      id: newId,
      data: { ...newChildData },
      children: [],
      collapsed: false,
    }

    // 更新树结构
    sourceNode.children = sourceNode.children || []
    sourceNode.children.push(newChildNode)

    // 更新原始数据
    sourceNode.data = sourceNode.data || {}
    sourceNode.data.children = sourceNode.data.children || []
    sourceNode.data.children.push(newChildData)

    // 展开父节点
    sourceNode.collapsed = false
    renderRef.current?.()
    onNodeChange?.(getMindNodeData(treeDataRef.current), 'create', newChildData)

    // 如果是通过快捷键创建的，需要选中新节点并进入编辑模式
    if (parentId && nodeValue) {
      setTimeout(() => {
        const graph = graphRef.current
        if (graph) {
          const cell = graph.getCellById(newId)
          if (cell && cell.isNode()) {
            graph.resetSelection(cell)
            graph.centerCell(cell)
            // 触发编辑状态，需要 AlgoNode 组件配合监听
            cell.setData({
              ...cell.getData(),
              editing: true,
            })
          }
        }
      }, 100)
    }
  }

  /**
   * 打开创建节点弹窗（实际上直接创建默认节点）
   */
  const openCreateDialog = (parentId: string) => {
    applyCreateChild(parentId, '新节点')
  }

  /**
   * 打开删除确认弹窗
   */
  const openDeleteDialog = (nodeId: string) => {
    if (treeDataRef.current?.id === nodeId) { return }
    const sourceNode = findNodeInTree(treeDataRef.current, nodeId)
    if (!sourceNode) { return }
    setDeleteNodeId(nodeId)
    setDeleteNodeName(sourceNode.data?.name ?? '')
    setDeleteOpen(true)
  }

  /**
   * 执行重命名操作
   */
  const applyRename = () => {
    if (!renameNodeId) { return }
    const nextName = renameValue.trim()
    if (!nextName) { return }
    const sourceNode = findNodeInTree(treeDataRef.current, renameNodeId)
    if (!sourceNode) { return }
    sourceNode.data = sourceNode.data || {}
    sourceNode.data.name = nextName
    setRenameOpen(false)
    setRenameNodeId(null)
    renderRef.current?.()
    onNodeChange?.(getMindNodeData(treeDataRef.current), 'update', sourceNode)
  }

  /**
   * 执行删除操作
   */
  const applyDelete = () => {
    if (!deleteNodeId) { return }
    if (treeDataRef.current?.id === deleteNodeId) { return }
    const found = findNodeWithParent(treeDataRef.current, deleteNodeId)
    if (!found || !found.parent) { return }

    const { node: deletedNode, parent, index } = found
    const deletedNodeName = deletedNode.data?.name || ''

    // 从树结构中移除
    parent.children.splice(index, 1)

    // 从原始数据结构中移除
    if (parent.data?.children) {
      parent.data.children = parent.data.children.filter((c: any) => c?.id !== deleteNodeId)
    }

    if (selectedNodeIdRef.current === deleteNodeId) {
      selectedNodeIdRef.current = null
    }

    setDeleteOpen(false)
    setDeleteNodeId(null)
    renderRef.current?.()
    onNodeChange?.(getMindNodeData(treeDataRef.current), 'delete', deletedNode)

    toast(`已删除 ${deletedNodeName}`, {
      action: {
        label: '撤回',
        onClick: () => {
          // 恢复树结构
          parent.children.splice(index, 0, deletedNode)

          // 恢复原始数据结构
          if (parent.data) {
            parent.data.children = parent.data.children || []
            // 假设索引匹配，我们将其拼接回去
            parent.data.children.splice(index, 0, deletedNode.data)
          }

          renderRef.current?.()
          onNodeChange?.(getMindNodeData(treeDataRef.current), 'undo', deletedNode)
        },
      },
    })
  }

  /**
   * 创建同级节点
   * @param nodeId 当前节点ID
   * @param nodeValue 新节点名称
   */
  const applyCreateSibling = (nodeId: string, nodeValue: string) => {
    const nextName = nodeValue.trim()
    if (!nextName) { return }

    const found = findNodeWithParent(treeDataRef.current, nodeId)
    if (!found || !found.parent) { return }

    const { parent, index } = found

    const newId = Math.random().toString(36).slice(2)
    const newChildData = {
      id: newId,
      name: nextName,
      children: [],
    }
    const newChildNode = {
      id: newId,
      data: { ...newChildData },
      children: [],
      collapsed: false,
    }

    // 在当前节点后插入
    parent.children = parent.children || []
    parent.children.splice(index + 1, 0, newChildNode)

    parent.data = parent.data || {}
    parent.data.children = parent.data.children || []
    parent.data.children.splice(index + 1, 0, newChildData)

    renderRef.current?.()
    onNodeChange?.(getMindNodeData(treeDataRef.current), 'create', newChildData)

    setTimeout(() => {
      const graph = graphRef.current
      if (graph) {
        const cell = graph.getCellById(newId)
        if (cell && cell.isNode()) {
          graph.resetSelection(cell)
          graph.centerCell(cell)
          cell.setData({
            ...cell.getData(),
            editing: true,
          })
        }
      }
    }, 100)
  }

  /**
   * 移动节点（拖拽重组）
   * @param draggedNodeId 被拖拽的节点ID
   * @param targetNodeId 目标父节点ID
   * @param position 移动位置：'before' | 'after' | 'child'
   */
  const applyMoveNode = (draggedNodeId: string, targetNodeId: string, position: 'before' | 'after' | 'child' = 'child') => {
    if (draggedNodeId === targetNodeId) { return }
    const draggedResult = findNodeWithParent(treeDataRef.current, draggedNodeId)
    const targetResult = findNodeWithParent(treeDataRef.current, targetNodeId)
    const targetNode = findNodeInTree(treeDataRef.current, targetNodeId)

    if (!draggedResult || !targetNode) { return }

    // 检查目标是否是被拖拽节点的后代（防止循环引用）
    if (findNodeInTree(draggedResult.node, targetNodeId)) {
      return // Cannot move parent into its own child
    }

    // 从旧父节点移除
    const { parent: oldParent, index: oldIndex } = draggedResult
    oldParent.children.splice(oldIndex, 1)
    if (oldParent.data && oldParent.data.children) {
      oldParent.data.children = oldParent.data.children.filter((c: any) => c.id !== draggedNodeId)
    }

    // 根据 position 添加到新位置
    if (position === 'child') {
      // 添加为子节点
      targetNode.children = targetNode.children || []
      targetNode.children.push(draggedResult.node)

      targetNode.data = targetNode.data || {}
      targetNode.data.children = targetNode.data.children || []
      targetNode.data.children.push(draggedResult.node.data)

      // 如果目标节点是折叠状态，则展开
      targetNode.collapsed = false
      targetNode.data.collapsed = false
    }
    else if (position === 'before' || position === 'after') {
      // 添加为同级节点
      // 如果目标节点是根节点，则无法添加同级节点（通常思维导图只有一个根）
      if (!targetResult || !targetResult.parent) {
        // 如果无法找到父节点（例如目标是根节点），则回退为添加到子节点
        // 或者直接返回
        console.warn('Cannot move to sibling of root node')

        // 恢复节点到原位置 (简单的回滚逻辑)
        oldParent.children.splice(oldIndex, 0, draggedResult.node)
        if (oldParent.data) {
          oldParent.data.children = oldParent.data.children || []
          oldParent.data.children.splice(oldIndex, 0, draggedResult.node.data)
        }
        return
      }

      const { parent: newParent, index: targetIndex } = targetResult
      const insertIndex = position === 'before' ? targetIndex : targetIndex + 1

      newParent.children = newParent.children || []
      newParent.children.splice(insertIndex, 0, draggedResult.node)

      newParent.data = newParent.data || {}
      newParent.data.children = newParent.data.children || []
      newParent.data.children.splice(insertIndex, 0, draggedResult.node.data)
    }

    renderRef.current?.()
    onNodeChange?.(getMindNodeData(treeDataRef.current), 'move', draggedResult.node)

    // 移动后聚焦到该节点
    setTimeout(() => {
      const graph = graphRef.current
      if (graph) {
        const cell = graph.getCellById(draggedNodeId)
        if (cell && cell.isNode()) {
          graph.resetSelection(cell)
          graph.centerCell(cell)
        }
      }
    }, 100)
  }

  return {
    renameOpen,
    setRenameOpen,
    renameNodeId,
    setRenameNodeId,
    renameValue,
    setRenameValue,
    deleteOpen,
    setDeleteOpen,
    deleteNodeId,
    setDeleteNodeId,
    deleteNodeName,
    setDeleteNodeName,
    openRenameDialog,
    openCreateDialog,
    openDeleteDialog,
    applyRename,
    applyDelete,
    applyCreateChild,
    applyCreateSibling,
    applyMoveNode,
  }
}
