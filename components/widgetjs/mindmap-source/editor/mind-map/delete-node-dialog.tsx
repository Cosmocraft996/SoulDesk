import React from 'react'
import { Button } from '../components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../components/ui/dialog'

interface DeleteNodeDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  nodeName: string
  onConfirm: () => void
}

export const DeleteNodeDialog: React.FC<DeleteNodeDialogProps> = ({
  open,
  onOpenChange,
  nodeName,
  onConfirm,
}) => {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>删除节点</DialogTitle>
        </DialogHeader>
        <div className="text-sm text-muted-foreground">
          确认删除“
          {nodeName || '未命名'}
          ”吗？该操作不可撤销。
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button variant="outline">取消</Button>
          </DialogClose>
          <Button variant="destructive" onClick={onConfirm}>
            删除
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
