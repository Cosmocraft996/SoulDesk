import type { Edge } from '@antv/x6'
import type { MindNode, MindNodeChangeType } from '../types/MindNode'
import Hierarchy from '@antv/hierarchy'
import { Export, Graph, Selection } from '@antv/x6'
import { useEffect, useRef } from 'react'
import { ThemeStyles } from '../constants/flow-constants'
import { registerCustomEdge } from './registry'
import { useGraphDnD } from './use-graph-dnd'
import { useGraphEvents } from './use-graph-events'
import { findNodeInTree, getVisibleData, transformData } from './utils'

interface UseGraphInitProps {
  containerRef: React.RefObject<HTMLDivElement | null>
  graphRef: React.RefObject<Graph | null>
  treeDataRef: React.RefObject<any>
  renderRef: React.RefObject<(() => void) | null>
  selectedNodeIdRef: React.RefObject<string | null>
  themeRef: React.RefObject<'light' | 'dark'>
  data: MindNode
  isDarkMode: boolean
  showCheckboxes: boolean
  showGrid: boolean
  readonly: boolean
  onNodeChange?: (data: MindNode, type: MindNodeChangeType, changedNode?: MindNode) => void
  openCreateDialog: (parentId: string) => void
  openDeleteDialog: (nodeId: string) => void
  openRenameDialog: (nodeId: string) => void
  applyMoveNode: (draggedNodeId: string, targetNodeId: string, position?: 'before' | 'after' | 'child') => void
}

export function useGraphInit({
  containerRef,
  graphRef,
  treeDataRef,
  renderRef,
  selectedNodeIdRef,
  themeRef,
  data,
  isDarkMode,
  showCheckboxes,
  showGrid,
  readonly,
  onNodeChange,
  openCreateDialog,
  openDeleteDialog,
  openRenameDialog,
  applyMoveNode,
}: UseGraphInitProps) {
  const lastRootIdRef = useRef<string | null>(null)

  // Update theme ref and graph styles when isDarkMode changes
  useEffect(() => {
    themeRef.current = isDarkMode ? 'dark' : 'light'
    const theme = ThemeStyles[themeRef.current]
    const graph = graphRef.current

    if (graph) {
      graph.drawBackground({ color: theme.bgColor })
      graph.drawGrid({
        args: [
          { color: theme.gridPrimary, thickness: 1 },
          { color: theme.gridSecondary, thickness: 1, factor: 4 },
        ] as any,
      })

      if (showGrid) {
        graph.showGrid()
      }
      else {
        graph.hideGrid()
      }

      // Update existing edges
      const edges = graph.getEdges()
      edges.forEach((edge) => {
        const currentStroke = edge.attr('line/stroke')
        if (currentStroke !== '#1890ff') {
          edge.attr('line/stroke', theme.lineColor)
        }
      })

      // Update default edge config for new edges
      registerCustomEdge(themeRef.current)
    }
  }, [isDarkMode, showGrid])

  // Initialize graph
  useEffect(() => {
    if (!containerRef.current) { return }

    registerCustomEdge(themeRef.current)

    const graph: Graph = new Graph({
      container: containerRef.current,
      autoResize: true,
      background: {
        color: ThemeStyles[themeRef.current].bgColor,
      },
      grid: {
        visible: true,
        type: 'doubleMesh',
        args: [
          {
            color: ThemeStyles[themeRef.current].gridPrimary,
            thickness: 1,
          },
          {
            color: ThemeStyles[themeRef.current].gridSecondary,
            thickness: 1,
            factor: 4,
          },
        ],
      },
      panning: true,
      mousewheel: true,
      connecting: {
        snap: true,
        allowBlank: false,
        allowLoop: false,
        highlight: true,
        connector: 'algo-connector',
        connectionPoint: 'anchor',
        anchor: 'center',
        createEdge(): Edge {
          return graph.createEdge({
            shape: 'dag-edge',
            attrs: {
              line: {
                strokeDasharray: '5 5',
              },
            },
            zIndex: -1,
          })
        },
      },
    })

    graphRef.current = graph
    graph.use(new Export())
    graph.use(new Selection({ enabled: true, multiple: false, rubberband: false }))

    graph.on('selection:changed', ({ selected }) => {
      const node = selected.find(c => c.isNode())
      selectedNodeIdRef.current = node ? node.id : null
    })

    // Initialize tree data
    treeDataRef.current = transformData(data)

    const render = () => {
      const root = treeDataRef.current

      const visibleData = getVisibleData(root)

      // 计算布局
      const result = Hierarchy.mindmap(visibleData, {
        direction: 'H',
        getHeight: () => 36, // Match AlgoNode height
        getWidth: () => 180, // Match AlgoNode width
        getHGap: () => 60,
        getVGap: () => 10,
        getSide: () => 'right',
      })

      const cells: any[] = []
      const traverse = (node: any) => {
        if (node) {
          const sourceNode = findNodeInTree(treeDataRef.current, node.id)
          const hasChildren = sourceNode && sourceNode.children && sourceNode.children.length > 0

          const dagNode = graph.createNode({
            id: node.id,
            shape: 'dag-node',
            x: node.x,
            y: node.y,
            data: {
              label: node.data.data.name,
              checked: sourceNode?.checked || false,
              collapsed: sourceNode?.collapsed || false,
              hasChildren,
              showCheckboxes,
              readonly,
              onAddChild: () => openCreateDialog(node.id),
              onDeleteNode: () => openDeleteDialog(node.id),
              onEditNode: () => openRenameDialog(node.id),
            },
            ports: [
              { id: 'left', group: 'left' },
              { id: 'right', group: 'right' },
            ],
          })
          cells.push(dagNode)

          if (node.children) {
            node.children.forEach((child: any) => {
              traverse(child)
              const edge = graph.createEdge({
                id: `${node.id}-${child.id}`,
                source: { cell: node.id, port: 'right' },
                target: { cell: child.id, port: 'left' },
                shape: 'dag-edge',
                connector: { name: 'algo-connector' },
                attrs: {
                  line: {
                    strokeDasharray: '5 5',
                  },
                },
              })
              cells.push(edge)
            })
          }
        }
      }

      traverse(result)

      // 使用 diff 更新图表，避免全量重新渲染
      graph.batchUpdate(() => {
        const existingNodes = graph.getNodes()
        const existingEdges = graph.getEdges()
        const existingNodeMap = new Map(existingNodes.map(node => [node.id, node]))
        const existingEdgeMap = new Map(existingEdges.map(edge => [edge.id, edge]))

        // 1. 更新或添加节点/边
        cells.forEach((cell) => {
          if (cell.isNode()) {
            const existingNode = existingNodeMap.get(cell.id)
            if (existingNode) {
              // 更新位置
              const currentPos = existingNode.getPosition()
              if (currentPos.x !== cell.getPosition().x || currentPos.y !== cell.getPosition().y) {
                existingNode.setPosition(cell.getPosition())
              }
              // 更新数据
              const currentData = existingNode.getData()
              const newData = cell.getData()
              // 简单的浅比较，如果需要更深层的比较可以自行实现
              if (JSON.stringify(currentData) !== JSON.stringify(newData)) {
                existingNode.setData(newData)
              }
              // 从待删除列表中移除
              existingNodeMap.delete(cell.id)
            }
            else {
              graph.addNode(cell)
            }
          }
          else if (cell.isEdge()) {
            const existingEdge = existingEdgeMap.get(cell.id)
            if (existingEdge) {
              // 边通常只需要存在即可，如果属性有变化可以在这里更新
              // 从待删除列表中移除
              existingEdgeMap.delete(cell.id)
            }
            else {
              graph.addEdge(cell)
            }
          }
        })

        // 2. 删除不再存在的节点和边
        existingNodeMap.forEach((node) => {
          graph.removeNode(node)
        })
        existingEdgeMap.forEach((edge) => {
          graph.removeEdge(edge)
        })
      })

      if (selectedNodeIdRef.current) {
        const cell = graph.getCellById(selectedNodeIdRef.current)
        if (cell && cell.isNode()) {
          // 保持选中状态，不需要重置，除非之前的选择丢失了
          if (!graph.isSelected(cell)) {
            graph.resetSelection(cell)
          }
        }
        else {
          graph.cleanSelection()
        }
      }
      // 只有在初始化或者加载新图表时才居中
      const rootId = root?.id
      if (rootId && rootId !== lastRootIdRef.current) {
        lastRootIdRef.current = rootId
        graph.centerContent()
      }
    }

    renderRef.current = render

    render()

    return () => {
      renderRef.current = null
      graphRef.current = null
      lastRootIdRef.current = null
      graph.dispose()
    }
  }, [data])

  // Use separated hooks for logic
  useGraphEvents({
    graphRef,
    treeDataRef,
    renderRef,
    onNodeChange,
    themeRef,
    data,
  })

  useGraphDnD({
    graphRef,
    treeDataRef,
    readonly,
    applyMoveNode,
    data,
  })

  // Update nodes when checkboxes/readonly state changes
  useEffect(() => {
    const graph = graphRef.current
    if (graph) {
      const nodes = graph.getNodes()
      nodes.forEach((node) => {
        const nodeData = node.getData()
        node.setData({
          ...nodeData,
          showCheckboxes,
          readonly,
        })
      })
    }
  }, [showCheckboxes, readonly])
}
