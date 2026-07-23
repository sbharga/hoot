<script lang="ts">
  import { onMount } from 'svelte';
  export let deadlineMs: number;
  export let serverOffsetMs = 0;
  let remainingMs = 0;
  let totalMs = 1;
  let lastDeadline: number | undefined;
  let timer: number;

  const update = () => remainingMs = Math.max(0, deadlineMs - (Date.now() + serverOffsetMs));

  onMount(() => {
    timer = window.setInterval(update, 100);
    return () => window.clearInterval(timer);
  });

  $: if (deadlineMs !== lastDeadline) {
    lastDeadline = deadlineMs;
    totalMs = Math.max(1, deadlineMs - (Date.now() + serverOffsetMs));
    update();
  }

  $: seconds = Math.ceil(remainingMs / 1000);
  $: progress = Math.max(0, Math.min(100, (remainingMs / totalMs) * 100));
  $: urgent = seconds <= 5;
</script>

<div
  class="countdown-bar"
  class:urgent
  role="progressbar"
  aria-label={`${seconds} seconds remaining`}
  aria-valuenow={seconds}
  aria-valuemin={0}
  aria-valuemax={Math.ceil(totalMs / 1000)}
>
  <div class="countdown-bar-fill" style="width: {progress}%"></div>
</div>
