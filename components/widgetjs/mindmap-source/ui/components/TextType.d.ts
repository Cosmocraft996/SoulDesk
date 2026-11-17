import type React from 'react'

export interface TextTypeProps extends React.HTMLAttributes<HTMLDivElement> {
  text: string | string[]
  as?: React.ElementType
  typingSpeed?: number
  initialDelay?: number
  pauseDuration?: number
  deletingSpeed?: number
  loop?: boolean
  showCursor?: boolean
  hideCursorWhileTyping?: boolean
  cursorCharacter?: string
  cursorClassName?: string
  cursorBlinkDuration?: number
  textColors?: string[]
  variableSpeed?: { min: number, max: number }
  onSentenceComplete?: (text: string, index: number) => void
  startOnVisible?: boolean
  reverseMode?: boolean
}

declare const TextType: React.FC<TextTypeProps>
export default TextType
