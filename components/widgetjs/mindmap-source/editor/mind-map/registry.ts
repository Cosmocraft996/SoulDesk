import { Graph, Path } from '@antv/x6'
import { ThemeStyles } from '../constants/flow-constants'

export function registerCustomConnector() {
  // Check if already registered to avoid warnings if strict
  // But X6 registry usually overwrites or throws if duplicates without overwrite flag?
  // The original code passed `true` as the third argument which is 'force' (overwrite).

  Graph.registerConnector(
    'algo-connector',
    (s, e) => {
      const offset = 4
      const deltaX = Math.abs(e.x - s.x)
      const control = Math.floor((deltaX / 3) * 2)

      const v1 = { x: s.x + offset + control, y: s.y }
      const v2 = { x: e.x - offset - control, y: e.y }

      return Path.parse(
        `
      M ${s.x} ${s.y}
      L ${s.x + offset} ${s.y}
      C ${v1.x} ${v1.y} ${v2.x} ${v2.y} ${e.x - offset} ${e.y}
      L ${e.x} ${e.y}
    `,
      ).serialize()
    },
    true,
  )
}

export function registerCustomEdge(theme: 'light' | 'dark' = 'light') {
  const styles = ThemeStyles[theme]
  Graph.registerEdge(
    'dag-edge',
    {
      inherit: 'edge',
      attrs: {
        line: {
          stroke: styles.lineColor,
          strokeWidth: 2,
          targetMarker: null,
        },
      },
    },
    true,
  )
}
