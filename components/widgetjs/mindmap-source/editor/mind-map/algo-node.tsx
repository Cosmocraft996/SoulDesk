import type { Cell } from '@antv/x6'
import { register } from '@antv/x6-react-shape'
import { Pencil, Plus, Trash2 } from 'lucide-react'
import React from 'react'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '../components/ui/context-menu'

export interface NodeStatus {
  id: string
  label?: string
  checked?: boolean
  collapsed?: boolean
  editing?: boolean
  readonly?: boolean
  depth?: number
  shouldAnimate?: boolean
}

function AlgoNode(props: { node: Cell }) {
  const { node } = props
  const data = node?.getData() as NodeStatus
  const { label, checked = false, collapsed = false, readonly = false, depth = 0, shouldAnimate = false } = data
  const showCheckboxes = (data as any).showCheckboxes !== false
  const onAddChild = (data as any).onAddChild as undefined | (() => void)
  const onDeleteNode = (data as any).onDeleteNode as undefined | (() => void)
  const onEditNode = (data as any).onEditNode as undefined | (() => void)

  const [editing, setEditing] = React.useState(data.editing || false)
  const [tempLabel, setTempLabel] = React.useState(label || '')
  const inputRef = React.useRef<HTMLInputElement>(null)

  React.useEffect(() => {
    if (data.editing !== undefined) {
      setEditing(data.editing)
    }
  }, [data.editing])

  React.useEffect(() => {
    setTempLabel(label || '')
  }, [label])

  React.useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [editing])

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    // Force update the data
    const isChecked = e.target.checked
    node.setData({
      ...data,
      checked: isChecked,
    })
  }

  const handleCheckboxClick = (e: React.MouseEvent) => {
    e.stopPropagation()
  }

  const handleCollapse = (e: React.MouseEvent) => {
    e.stopPropagation()
    node.setData({
      ...data,
      collapsed: !collapsed,
    })
  }

  const handleLabelDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (readonly) { return }
    setEditing(true)
  }

  const handleLabelChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setTempLabel(e.target.value)
  }

  const handleLabelBlur = () => {
    setEditing(false)
    if (tempLabel !== label) {
      node.setData({
        ...data,
        label: tempLabel,
        editing: false,
      })
    }
    else {
      node.setData({
        ...data,
        editing: false,
      })
    }
  }

  const handleLabelKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleLabelBlur()
    }
  }

  // Check if node has children in the data (not just visible graph edges)
  // We'll pass a hasChildren flag in data
  const hasChildren = (data as any).hasChildren

  const content = (
    <div
      className={`node ${checked ? 'checked' : ''} ${shouldAnimate ? 'node-enter' : ''}`}
      style={{
        animationDelay: shouldAnimate ? `${depth * 50}ms` : '0ms',
      }}
    >
      {showCheckboxes && (
        <span className="checkbox-wrapper">
          <input
            type="checkbox"
            checked={checked}
            disabled={readonly}
            onChange={handleChange}
            onClick={handleCheckboxClick}
            onMouseDown={handleCheckboxClick}
            style={{ pointerEvents: 'auto' }}
          />
        </span>
      )}
      {editing
        ? (
            <input
              ref={inputRef}
              className="label-input"
              value={tempLabel}
              onChange={handleLabelChange}
              onBlur={handleLabelBlur}
              onKeyDown={handleLabelKeyDown}
              onMouseDown={e => e.stopPropagation()}
            />
          )
        : (
            <span className="label" onDoubleClick={handleLabelDoubleClick}>
              {label}
            </span>
          )}
      <span className="status">
        {hasChildren && (
          <button
            className="collapse-btn"
            onClick={handleCollapse}
            onMouseDown={e => e.stopPropagation()}
          >
            {collapsed ? '+' : '-'}
          </button>
        )}
      </span>
    </div>
  )

  if (readonly) {
    return content
  }

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        {content}
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          onSelect={(e) => {
            e.preventDefault()
            onAddChild?.()
          }}
        >
          <Plus />
          添加
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={(e) => {
            e.preventDefault()
            onDeleteNode?.()
          }}
        >
          <Trash2 />
          删除
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={(e) => {
            e.preventDefault()
            onEditNode?.()
          }}
        >
          <Pencil />
          修改
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

register({
  shape: 'dag-node',
  width: 180,
  height: 36,
  component: AlgoNode,
  ports: {
    groups: {
      left: {
        position: 'left',
        attrs: {
          circle: {
            r: 0,
            magnet: true,
            stroke: 'transparent',
            strokeWidth: 0,
            fill: 'transparent',
          },
        },
      },
      right: {
        position: 'right',
        attrs: {
          circle: {
            r: 0,
            magnet: true,
            stroke: 'transparent',
            strokeWidth: 0,
            fill: 'transparent',
          },
        },
      },
    },
  },
})

export default AlgoNode
