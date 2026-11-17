const images = import.meta.glob('./assets/*.png', { eager: true, query: '?url', import: 'default' }) as Record<string, string>;
export function weatherImage(code: unknown) {
  const key = typeof code === 'string' && /^\d{3,4}$/.test(code) ? code : '999';
  return images[`./assets/${key}.png`] || images['./assets/999.png'];
}
