import type React from 'react'
import { useEffect } from 'react'
import { graphShortcutAllowed } from '../../keyboard-guard.js'

interface UseKeyboardShortcutsProps {
  renameOpen: boolean
  readonly: boolean
  selectedNodeIdRef: React.MutableRefObject<string | null>
  applyCreateSibling: (id: string, name: string) => void
  applyCreateChild: (id: string, name: string) => void
  openDeleteDialog: (id: string) => void
}

export function useKeyboardShortcuts({
  renameOpen,
  readonly,
  selectedNodeIdRef,
  applyCreateSibling,
  applyCreateChild,
  openDeleteDialog,
}: UseKeyboardShortcutsProps) {
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!graphShortcutAllowed(e)) { return }
      if (renameOpen) { return }

      const target = e.target as HTMLElement | null
      if (target) {
        const tagName = target.tagName
        if (tagName === 'INPUT' || tagName === 'TEXTAREA' || target.isContentEditable) {
          return
        }
      }

      const selectedNodeId = selectedNodeIdRef.current
      if (!selectedNodeId) { return }

      if (readonly) { return }

      if (e.key === 'Enter') {
        e.preventDefault()
        applyCreateSibling(selectedNodeId, '新节点')
      }

      if (e.key === 'Tab') {
        e.preventDefault()
        applyCreateChild(selectedNodeId, '新节点')
      }

      if (e.key === 'Delete') {
        e.preventDefault()
        openDeleteDialog(selectedNodeId)
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [renameOpen, readonly, selectedNodeIdRef, applyCreateSibling, applyCreateChild, openDeleteDialog])
}
