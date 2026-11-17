import type { Cell, Edge, Graph, Node } from '@antv/x6'
import type { MindNode, MindNodeChangeType } from '../types/MindNode'
import type { NodeStatus } from './algo-node'
import { useEffect } from 'react'
import { ThemeStyles } from '../constants/flow-constants'
import { findNodeInTree, getMindNodeData } from './utils'

interface UseGraphEventsProps {
  graphRef: React.RefObject<Graph | null>
  treeDataRef: React.RefObject<any>
  renderRef: React.RefObject<(() => void) | null>
  onNodeChange?: (data: MindNode, type: MindNodeChangeType, changedNode?: MindNode) => void
  themeRef: React.RefObject<'light' | 'dark'>
  data: any // Dependency to trigger re-bind when graph is re-created
}

export function useGraphEvents({
  graphRef,
  treeDataRef,
  renderRef,
  onNodeChange,
  themeRef,
  data,
}: UseGraphEventsProps) {
  useEffect(() => {
    const graph = graphRef.current
    if (!graph) { return }

    // 监听数据变化事件
    const onNodeChangeData = ({ node, current, previous }: { node: Node, current: any, previous: any }) => {
      const sourceNode = findNodeInTree(treeDataRef.current, node.id)
      if (!sourceNode) { return }

      // Handle collapsed change
      if (current.collapsed !== previous?.collapsed) {
        sourceNode.collapsed = current.collapsed

        // If collapsing, reset animation state for all children so they animate again when expanded
        if (sourceNode.collapsed) {
          const resetAnimated = (n: any) => {
            if (n.children) {
              n.children.forEach((c: any) => {
                c._animated = false
                resetAnimated(c)
              })
            }
          }
          resetAnimated(sourceNode)
        }

        // Re-render the graph
        if (renderRef.current) {
          renderRef.current()
        }
        onNodeChange?.(getMindNodeData(treeDataRef.current), 'collapse', sourceNode)
        return // Return early since render() will reset cells
      }

      // Handle checked change
      if (current.checked !== previous?.checked) {
        const isChecked = current.checked
        sourceNode.checked = isChecked // Update source of truth

        const updateChildrenChecked = (n: any, checked: boolean) => {
          if (n.children) {
            n.children.forEach((c: any) => {
              c.checked = checked
              updateChildrenChecked(c, checked)
            })
          }
        }
        updateChildrenChecked(sourceNode, isChecked)

        const outgoingEdges = graph.getOutgoingEdges(node)
        if (outgoingEdges) {
          outgoingEdges.forEach((edge: Edge) => {
            const childNode = edge.getTargetCell()
            if (childNode && childNode.isNode()) {
              const childData = childNode.getData() as NodeStatus
              if (childData.checked !== isChecked) {
                childNode.setData({
                  ...childData,
                  checked: isChecked,
                })
              }
            }
          })
        }
        onNodeChange?.(getMindNodeData(treeDataRef.current), 'check', sourceNode)
      }

      // Handle label change
      if (current.label !== previous?.label) {
        sourceNode.data = sourceNode.data || {}
        sourceNode.data.name = current.label
        onNodeChange?.(getMindNodeData(treeDataRef.current), 'update', sourceNode)
      }
    }

    // 鼠标移入节点时高亮路径
    const onNodeMouseEnter = ({ node }: { node: Node }) => {
      // 找到回溯到根节点的路径
      const edges: Cell[] = []
      let current = node
      while (current) {
        const incomingEdges = graph.getIncomingEdges(current)
        if (incomingEdges && incomingEdges.length > 0) {
          const edge = incomingEdges[0] // 树状结构只有一个入边
          edges.push(edge)
          current = edge.getSourceCell() as any
        }
        else {
          break
        }
      }

      edges.forEach((edge) => {
        edge.attr('line/stroke', '#1890ff')
        edge.attr('line/strokeDasharray', 5)
        edge.attr('line/style/animation', 'ant-line-flow 30s linear infinite')
      })
    }

    // 鼠标移出节点时恢复路径
    const onNodeMouseLeave = ({ node }: { node: Node }) => {
      // 找到回溯到根节点的路径
      const edges: Cell[] = []
      let current = node
      while (current) {
        const incomingEdges = graph.getIncomingEdges(current)
        if (incomingEdges && incomingEdges.length > 0) {
          const edge = incomingEdges[0] // 树状结构只有一个入边
          edges.push(edge)
          current = edge.getSourceCell() as any
        }
        else {
          break
        }
      }

      const theme = themeRef.current
      edges.forEach((edge) => {
        edge.attr('line/stroke', ThemeStyles[theme].lineColor)
        edge.attr('line/strokeDasharray', '5 5') // 恢复为虚线，或者 '' 如果是实线
        edge.attr('line/style/animation', '')
      })
    }

    graph.on('node:change:data', onNodeChangeData)
    graph.on('node:mouseenter', onNodeMouseEnter)
    graph.on('node:mouseleave', onNodeMouseLeave)

    return () => {
      graph.off('node:change:data', onNodeChangeData)
      graph.off('node:mouseenter', onNodeMouseEnter)
      graph.off('node:mouseleave', onNodeMouseLeave)
    }
  }, [graphRef, treeDataRef, renderRef, onNodeChange, themeRef, data])
}
