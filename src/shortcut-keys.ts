import type { KeyboardEvent } from 'react';

// 与宿主的 Windows 快捷键解析器使用相同的键名。
const shortcutKeyCodes = new Set([
  'Backquote', 'Backslash', 'BracketLeft', 'BracketRight', 'Comma', 'Equal', 'Minus',
  'Period', 'Quote', 'Semicolon', 'Slash', 'Backspace', 'CapsLock', 'Enter', 'Space',
  'Tab', 'Escape', 'Delete', 'End', 'Home', 'Insert', 'PageDown', 'PageUp', 'PrintScreen',
  'ScrollLock', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'Pause', 'NumLock',
  'NumpadAdd', 'NumpadDecimal', 'NumpadDivide', 'NumpadEnter', 'NumpadEqual', 'NumpadMultiply', 'NumpadSubtract',
  'AudioVolumeDown', 'AudioVolumeUp', 'AudioVolumeMute', 'MediaPlay', 'MediaPause',
  'MediaPlayPause', 'MediaStop', 'MediaTrackNext', 'MediaTrackPrevious',
]);

export function shortcutMainKey({ code, key }: KeyboardEvent<HTMLInputElement>) {
  if (/^Key[A-Z]$/.test(code)) return /^[a-z]$/i.test(key) ? key.toUpperCase() : code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Numpad[0-9]$/.test(code) || code === 'NumpadDecimal') {
    // 数字锁关闭时，小键盘发送方向、删除和导航键。
    return shortcutKeyCodes.has(key) ? key : code;
  }
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code) || shortcutKeyCodes.has(code)) return code;
}

export function heldShortcutModifiers({ ctrlKey, altKey, shiftKey, metaKey }: KeyboardEvent<HTMLInputElement>) {
  return [ctrlKey && 'Ctrl', altKey && 'Alt', shiftKey && 'Shift', metaKey && 'Super'].filter(Boolean).join('+');
}
