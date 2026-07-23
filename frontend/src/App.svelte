<script lang="ts">
  import Host from './Host.svelte';
  import Player from './Player.svelte';

  const isHost = window.location.pathname === '/host' || window.location.pathname.startsWith('/host/');
</script>

{#if isHost}
  <Host />
{:else}
  <Player />
{/if}

