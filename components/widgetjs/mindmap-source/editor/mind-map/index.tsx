import type { Graph } from '@antv/x6'
import type { MindNode, MindNodeChangeType } from '../types/MindNode'
import { Lock } from 'lucide-react'
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { Kbd } from '../components/ui/kbd'
import { DeleteNodeDialog } from './delete-node-dialog'
import { MindMapToolbar } from './mind-map-toolbar'
import { registerCustomConnector } from './registry'
import { RenameNodeDialog } from './rename-node-dialog'
import { useGraphInit } from './use-graph-init'
import { useGraphOperations } from './use-graph-operations'
import { useKeyboardShortcuts } from './use-keyboard-shortcuts'
import './algo-node' // Import for side effects (registering shape and inserting css)
import './index.css'
import { createPng } from '../../png-export.js'

interface MindMapProps {
  data: MindNode
  isDarkMode?: boolean
  title?: string
  showCheckboxes?: boolean
  showGrid?: boolean
  readonly?: boolean
  onConfigChange?: (key: 'showCheckboxes' | 'showGrid', value: boolean) => void
  onNodeChange?: (data: MindNode, type: MindNodeChangeType, changedNode?: MindNode) => void
  onExport?: (uri: string, title: string) => Promise<void>
  onExportError?: (error: unknown) => void
}

export interface MindMapRef {
  exportGraph: () => void
}

// Register custom connector
registerCustomConnector()

export const MindMap = forwardRef<MindMapRef, MindMapProps>(({ data, isDarkMode = false, showCheckboxes: propShowCheckboxes, showGrid: propShowGrid, title = '思维导图', readonly = false, onNodeChange, onConfigChange, onExport, onExportError }, ref) => {
  const containerRef = useRef<HTMLDivElement>(null)
  const graphRef = useRef<Graph>(null)
  const treeDataRef = useRef<any>(null)
  const renderRef = useRef<null | (() => void)>(null)
  const selectedNodeIdRef = useRef<string | null>(null)
  const themeRef = useRef<'light' | 'dark'>(isDarkMode ? 'dark' : 'light')

  const [showCheckboxes, setShowCheckboxes] = useState(propShowCheckboxes ?? true)
  const [showGrid, setShowGrid] = useState(propShowGrid ?? true)
  const [isExporting, setIsExporting] = useState(false)

  useEffect(() => {
    if (propShowCheckboxes !== undefined) {
      setShowCheckboxes(propShowCheckboxes)
    }
  }, [propShowCheckboxes])

  useEffect(() => {
    if (propShowGrid !== undefined) {
      setShowGrid(propShowGrid)
    }
  }, [propShowGrid])

  const {
    renameOpen,
    setRenameOpen,
    setRenameNodeId,
    renameValue,
    setRenameValue,
    deleteOpen,
    setDeleteOpen,
    setDeleteNodeId,
    deleteNodeName,
    openCreateDialog,
    openDeleteDialog,
    openRenameDialog,
    applyRename,
    applyDelete,
    applyCreateChild,
    applyCreateSibling,
    applyMoveNode,
  } = useGraphOperations({
    graphRef,
    treeDataRef,
    renderRef,
    selectedNodeIdRef,
    onNodeChange,
  })

  useGraphInit({
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
  })

  useKeyboardShortcuts({
    renameOpen,
    readonly,
    selectedNodeIdRef,
    applyCreateSibling,
    applyCreateChild,
    openDeleteDialog,
  })

  const exportGraph = () => {
    const graph = graphRef.current
    if (graph) {
      setIsExporting(true)
      requestAnimationFrame(() => {
        const options = { padding: 20, backgroundColor: 'transparent', quality: 1 }
        if (onExport) {
          void createPng(graph).then(uri => onExport(uri as string, title)).catch(error => onExportError?.(error)).finally(() => setIsExporting(false))
        } else {
          graph.exportPNG(title.replace(/\s+/g, '-').replace(/\./g, '_').toLowerCase(), options)
          setIsExporting(false)
        }
      })
    }
  }

  useImperativeHandle(ref, () => ({
    exportGraph,
  }))

  const zoomIn = () => {
    const graph = graphRef.current
    if (graph) {
      graph.zoom(0.1)
    }
  }

  const zoomOut = () => {
    const graph = graphRef.current
    if (graph) {
      graph.zoom(-0.1)
    }
  }

  const zoomToOne = () => {
    const graph = graphRef.current
    if (graph) {
      graph.zoomToFit({ padding: 20 })
      graph.centerContent()
    }
  }

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%', maxHeight: '100vh' }}>
      <div
        ref={containerRef}
        data-mindmap-canvas
        tabIndex={0}
        onPointerDownCapture={event => {
          if (!(event.target as Element).closest('input,textarea,button')) {
            event.currentTarget.focus({ preventScroll: true })
          }
        }}
        style={{
          width: '100%',
          height: '100%',
        }}
      />
      <div className="mindmap-header">
        {title}
        {readonly && <Lock className="mindmap-header-icon" />}
      </div>
      <MindMapToolbar
        zoomIn={zoomIn}
        zoomOut={zoomOut}
        zoomToOne={zoomToOne}
        exportGraph={exportGraph}
        isExporting={isExporting}
        showCheckboxes={showCheckboxes}
        onToggleCheckboxes={(checked) => {
          setShowCheckboxes(checked)
          onConfigChange?.('showCheckboxes', checked)
        }}
        showGrid={showGrid}
        onToggleGrid={(checked) => {
          setShowGrid(checked)
          onConfigChange?.('showGrid', checked)
        }}
      />
      {!readonly && (
        <div className="mindmap-shortcuts">
          <div className="mindmap-shortcuts-title">选中节点后：</div>
          <div className="mindmap-shortcuts-list">
            <div className="mindmap-shortcut-item">
              <Kbd>Tab</Kbd>
              <span>创建子节点</span>
            </div>
            <div className="mindmap-shortcut-item">
              <Kbd>Enter</Kbd>
              <span>创建同级节点</span>
            </div>
            <div className="mindmap-shortcut-item">
              <Kbd>Delete</Kbd>
              <span>删除节点</span>
            </div>
          </div>
        </div>
      )}
      <RenameNodeDialog
        open={renameOpen}
        onOpenChange={(open) => {
          setRenameOpen(open)
          if (!open) {
            setRenameNodeId(null)
          }
        }}
        value={renameValue}
        onChange={setRenameValue}
        onConfirm={applyRename}
      />
      <DeleteNodeDialog
        open={deleteOpen}
        onOpenChange={(open) => {
          setDeleteOpen(open)
          if (!open) {
            setDeleteNodeId(null)
          }
        }}
        nodeName={deleteNodeName}
        onConfirm={applyDelete}
      />
    </div>
  )
})
