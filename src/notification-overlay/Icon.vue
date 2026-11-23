<script setup>
defineProps({type:String});
const paths={computer:'M3 4h18v13H3z M8 21h8 M12 17v4',success:'M5 12l4 4L19 6',error:'M6 6l12 12 M18 6L6 18',warning:'M12 3L2 21h20L12 3 M12 9v5 M12 17v1',info:'M12 10v7 M12 6v1',
  volume:'M3 9h4l5-4v14l-5-4H3z M16 9a4 4 0 0 1 0 6 M19 6a8 8 0 0 1 0 12',
  'volume-off':'M3 9h4l5-4v14l-5-4H3z M16 9l6 6 M22 9l-6 6'};
</script>
<template><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="1em" height="1em" aria-hidden="true"><circle v-if="['info','success','error'].includes(type)" cx="12" cy="12" r="10"/><path :d="paths[type]??paths.info"/></svg></template>
