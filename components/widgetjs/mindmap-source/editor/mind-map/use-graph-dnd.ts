import type { Cell, Graph } from '@antv/x6'
import { useEffect, useRef } from 'react'
import { findNodeInTree, getAllDescendants } from './utils'

interface UseGraphDnDProps {
  graphRef: React.RefObject<Graph | null>
  treeDataRef: React.RefObject<any>
  readonly: boolean
  applyMoveNode: (draggedNodeId: string, targetNodeId: string, position?: 'before' | 'after' | 'child') => void
  data: any // Dependency to trigger re-bind when graph is re-created
}

export function useGraphDnD({
  graphRef,
  treeDataRef,
  readonly,
  applyMoveNode,
  data,
}: UseGraphDnDProps) {
  const dragTargetNodeIdRef = useRef<string | null>(null)
  const dragPositionRef = useRef<'before' | 'after' | 'child' | null>(null)
  const indicatorRef = useRef<Cell | null>(null)
  const applyMoveNodeRef = useRef(applyMoveNode)

  // Keep applyMoveNodeRef current
  useEffect(() => {
    applyMoveNodeRef.current = applyMoveNode
  }, [applyMoveNode])

  useEffect(() => {
    const graph = graphRef.current
    if (!graph) { return }

    // 拖动节点时，检测重叠并高亮
    const onNodeMoving = ({ node, x: _x, y: _y }: { node: any, x: number, y: number }) => {
      if (readonly) { return }

      // 获取节点中心坐标（x, y 是左上角坐标）
      const { width, height } = node.getSize()
      const { x: nodeX, y: nodeY } = node.getPosition()
      // const centerX = x + width / 2
      // const centerY = y + height / 2
      const sourceBottom = nodeY + height
      // const sourceRight = nodeX + width

      // 查找当前区域内的所有节点（基于 Graph 坐标系）
      // 使用节点中心点的一个小区域来检测
      const nodes = graph.getNodesInArea(nodeX, nodeY, width, height)

      // 找到目标节点（排除自己）
      const targetNode = nodes.find(n => n.id !== node.id && n.isNode())

      // 检查目标节点是否有效（非后代节点）
      let isValidTarget = false
      let targetPosition: 'before' | 'after' | 'child' | null = null

      if (targetNode) {
        const sourceNodeData = findNodeInTree(treeDataRef.current, node.id)
        // 如果目标节点不是源节点的后代，则是有效目标
        if (sourceNodeData && !findNodeInTree(sourceNodeData, targetNode.id)) {
          isValidTarget = true

          // 计算目标节点中心点
          const { x: targetX, y: targetY } = targetNode.getPosition()
          const { width: targetWidth, height: targetHeight } = targetNode.getSize()
          const targetCenterX = targetX + targetWidth / 2
          const targetCenterY = targetY + targetHeight / 2

          // 判定逻辑：
          // child: 拖拽节点在目标节点中心点右侧 (draggedNode.x > targetNode.centerX)
          // before: 拖拽节点在目标节点中心点左侧且上方 (draggedNode.x < targetNode.centerX && draggedNode.y < tagetNode.centerY)
          // after: 拖拽节点在目标节点中心点左侧且下方 (draggedNode.x < targetNode.centerX && draggedNode.y > tagetNode.centerY)

          // 使用拖拽节点的中心点来判断会更自然一些
          if (nodeX > targetCenterX) {
            targetPosition = 'child'
          }
          else if (sourceBottom < targetCenterY) {
            targetPosition = 'before'
          }
          else {
            targetPosition = 'after'
          }
        }
      }

      // 如果之前的目标节点不再有效，或者位置发生了变化
      const hasActiveTarget = !!dragTargetNodeIdRef.current
      if (hasActiveTarget) {
        const isTargetInvalid = !targetNode
        const isTargetChanged = targetNode && targetNode.id !== dragTargetNodeIdRef.current
        const isTargetNotAllowed = !isValidTarget
        const isPositionChanged = dragPositionRef.current !== targetPosition

        if (isTargetInvalid || isTargetChanged || isTargetNotAllowed || isPositionChanged) {
          const prevTargetId = dragTargetNodeIdRef.current
          const prevTargetCell = graph.getCellById(prevTargetId!)
          if (prevTargetCell) {
            // 移除选中状态
            graph.unselect(prevTargetCell)
          }
          // 移除之前的指示器
          if (indicatorRef.current) {
            indicatorRef.current.remove()
            indicatorRef.current = null
          }
          dragTargetNodeIdRef.current = null
          dragPositionRef.current = null
        }
      }

      // 如果找到了新的有效目标节点
      if (targetNode && isValidTarget && targetPosition) {
        // 更新引用
        if (dragTargetNodeIdRef.current !== targetNode.id || dragPositionRef.current !== targetPosition) {
          dragTargetNodeIdRef.current = targetNode.id
          dragPositionRef.current = targetPosition

          // 使用选中状态来高亮目标节点
          graph.select(targetNode)

          // 创建/更新指示器 (半透明虚拟节点)
          if (indicatorRef.current) {
            indicatorRef.current.remove()
          }

          const { x: tx, y: ty, width: tw, height: th } = targetNode.getBBox()
          let indicatorX = tx
          let indicatorY = ty

          if (targetPosition === 'child') {
            indicatorX = tx + tw + 60 // 假设水平间距是 60
            indicatorY = ty // 简单对齐，实际上可能是子列表的末尾
          }
          else if (targetPosition === 'before') {
            indicatorX = tx
            indicatorY = ty - th - 10 // 假设垂直间距是 10
          }
          else if (targetPosition === 'after') {
            indicatorX = tx
            indicatorY = ty + th + 10
          }

          // 创建一个临时的指示节点
          indicatorRef.current = graph.createNode({
            shape: 'rect', // 或者使用 'dag-node' 如果你想保持一致的外观
            x: indicatorX,
            y: indicatorY,
            width: 180, // 假设宽度
            height: 36, // 假设高度
            attrs: {
              body: {
                fill: '#1890ff',
                stroke: '#1890ff',
                strokeDasharray: '5 5',
                opacity: 0.3,
                rx: 4,
                ry: 4,
              },
              label: {
                text: '新位置',
                fill: '#fff',
                fontSize: 12,
              },
            },
            pointerEvents: 'none', // 确保不干扰鼠标事件
          })

          graph.addCell(indicatorRef.current)
        }
      }
    }

    // 拖动结束，执行移动操作
    const onNodeMoved = ({ node }: { node: any }) => {
      if (readonly) { return }

      // 清理指示器
      if (indicatorRef.current) {
        indicatorRef.current.remove()
        indicatorRef.current = null
      }

      if (dragTargetNodeIdRef.current && dragPositionRef.current) {
        const targetId = dragTargetNodeIdRef.current
        const position = dragPositionRef.current
        const targetCell = graph.getCellById(targetId)

        // 清除选中样式
        if (targetCell) {
          graph.unselect(targetCell)
        }

        dragTargetNodeIdRef.current = null
        dragPositionRef.current = null

        // 执行移动逻辑
        applyMoveNodeRef.current(node.id, targetId, position)
      }
    }

    graph.on('node:moving', onNodeMoving)
    graph.on('node:moved', onNodeMoved)

    // 监听节点位置变化，实现子节点跟随移动
    const onNodeChangePosition = ({ node, current, previous, options }: { node: Cell, current: { x: number, y: number }, previous: { x: number, y: number }, options: any }) => {
      if (readonly) { return }

      // 只有当是用户拖拽(options.ui)或者显式指定需要跟随(options.isFollower !== true)时才处理
      // 注意：如果是代码触发的更新且没有标记为 isFollower，通常不应该触发跟随，除非明确是用户意图
      // X6 中用户拖拽触发的事件 options.ui 为 true
      if (options.ui && !options.isFollower) {
        const dx = current.x - previous.x
        const dy = current.y - previous.y

        if (dx === 0 && dy === 0) { return }

        // 查找所有子孙节点
        const descendants = getAllDescendants(treeDataRef.current, node.id)

        if (descendants.length > 0) {
          // 批量移动子节点
          graph.batchUpdate(() => {
            descendants.forEach((childId) => {
              const childNode = graph.getCellById(childId)
              if (childNode && childNode.isNode()) {
                // 使用 translate 移动，并标记 isFollower，避免循环触发
                childNode.translate(dx, dy, { ui: true, isFollower: true })
              }
            })
          })
        }
      }
    }

    // 使用 any 类型绕过 X6 事件类型定义的限制
    graph.on('node:change:position', onNodeChangePosition as any)

    return () => {
      graph.off('node:moving', onNodeMoving)
      graph.off('node:moved', onNodeMoved)
      graph.off('node:change:position', onNodeChangePosition as any)
    }
  }, [graphRef, treeDataRef, readonly, data])
}
