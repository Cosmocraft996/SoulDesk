export function graphShortcutAllowed(event){
  const target=event.target;
  return !event.defaultPrevented&&!event.ctrlKey&&!event.altKey&&!event.metaKey&&!!target?.closest?.('[data-mindmap-canvas]')&&!target.closest('[role="dialog"],dialog')&&!target.isContentEditable&&!['INPUT','TEXTAREA','BUTTON','SELECT'].includes(target.tagName);
}
