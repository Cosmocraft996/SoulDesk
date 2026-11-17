import { defineComponent, h } from 'vue';
export const LocalTwo = defineComponent({ setup: () => () => h('span', { 'aria-label':'地点', class:'location-symbol' }, '⌖') });
export const Tooltip = defineComponent({ props: { content:String }, setup: (props, { slots }) => () => h('span', { title:props.content }, slots.default?.()) });
