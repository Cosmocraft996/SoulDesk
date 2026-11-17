export interface MindNode {
  id: string
  name: string
  type?: string
  checked?: boolean
  /**
   * 技能节点的首页链接
   */
  url?: string
  collapsed?: boolean
  children?: MindNode[]
}

export type MindNodeChangeType = 'create' | 'delete' | 'update' | 'check' | 'collapse' | 'undo' | 'move'
