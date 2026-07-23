<script lang="ts">
  import { onMount } from 'svelte';
  import QRCode from 'qrcode';
  import Countdown from './lib/Countdown.svelte';
  import { connect } from './lib/connection';
  import type { Choice, HostState } from './lib/types';

  // Mirrors CSS --ink / --bg from styles.css; QRCode.js can't read CSS custom properties.
  const QR_INK = '#18181b';
  const QR_CREAM = '#ffffff';

  let state: HostState | undefined;
  let error = '';
  let recoveryKey = '';
  let needsRecovery = false;
  let connection: ReturnType<typeof connect> | undefined;
  let qrCode = '';
  let qrCodeUrl = '';
  let serverOffsetMs = 0;
  let serverOffsetSet = false;

  onMount(() => {
    const token = localStorage.getItem('hoot.hostToken');
    if (token) openConnection(token);
    else claimHost();
    return () => connection?.close();
  });

  async function claimHost(key?: string) {
    error = '';
    try {
      const response = await fetch('/api/host/claim', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ recoveryKey: key || null })
      });
      const body = await response.json();
      if (!response.ok) {
        needsRecovery = true;
        throw new Error(body.error || 'Host control could not be claimed.');
      }
      localStorage.setItem('hoot.hostToken', body.token);
      needsRecovery = false;
      openConnection(body.token);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Host control could not be claimed.';
    }
  }

  function openConnection(token: string) {
    connection?.close();
    serverOffsetSet = false;
    connection = connect('host', token, (next: HostState) => {
      state = next;
      if (!serverOffsetSet) {
        serverOffsetMs = next.serverTimeMs - Date.now();
        serverOffsetSet = true;
      }
      updateQrCode(next.joinUrl);
      error = '';
    }, (message, authenticationFailed) => {
      error = message;
      if (authenticationFailed) {
        localStorage.removeItem('hoot.hostToken');
        connection?.close();
        claimHost();
      }
    }, () => {});
  }

  function send(command: Record<string, unknown>) { connection?.send(command); }
  function phaseIs(...names: string[]) { return !!state && names.includes(state.phase.name); }
  function selectedGame() {
    const current = state;
    if (!current || current.phase.name !== 'lobby') return undefined;
    const gameId = current.phase.game_id;
    return current.games.find(game => game.id === gameId);
  }
  function deadline() {
    if (!state) return undefined;
    return 'deadline_ms' in state.phase ? state.phase.deadline_ms : undefined;
  }
  function answerSymbol(choice: Choice) {
    return choice.shape === 'triangle' ? '▲' : choice.shape === 'diamond' ? '◆' : choice.shape === 'circle' ? '●' : '■';
  }
  function rankMovement(delta: number) { return delta > 0 ? `↑ ${delta}` : delta < 0 ? `↓ ${Math.abs(delta)}` : '—'; }
  function optionCount(choice: Choice) {
    return state?.distribution?.options?.find((item: any) => item.id === choice.id)?.count || 0;
  }
  function optionSharePercent(choice: Choice) {
    const options = state?.distribution?.options;
    if (!options) return 0;
    const total = options.reduce((sum: number, item: any) => sum + (item.count || 0), 0);
    return total ? Math.round((optionCount(choice) / total) * 100) : 0;
  }

  function updateQrCode(url: string | undefined) {
    if (!url || url === qrCodeUrl) return;
    qrCodeUrl = url;
    QRCode.toDataURL(url, { width: 300, margin: 1, color: { dark: QR_INK, light: QR_CREAM } })
      .then(value => { if (qrCodeUrl === url) qrCode = value; })
      .catch(() => { if (qrCodeUrl === url) qrCode = ''; });
  }
</script>

<svelte:head><title>Host · Hoot!</title></svelte:head>

<main class="host-shell">
  {#if error}<div class="toast error" role="alert">{error}</div>{/if}

  {#if needsRecovery}
    <section class="center-card narrow">
      <p class="eyebrow">Host recovery</p>
      <h1>Take back the stage</h1>
      <p>Enter the recovery key printed in the Hoot server terminal. This signs out the previous host browser without changing the game.</p>
      <form on:submit|preventDefault={() => claimHost(recoveryKey)}>
        <label for="recovery">Recovery key</label>
        <input id="recovery" bind:value={recoveryKey} autocomplete="off" required />
        <button class="primary" type="submit">Take over host controls</button>
      </form>
    </section>
  {:else if !state}
    <section class="center-card"><div class="owl-loader" aria-label="Connecting">Connecting…</div><h1>Warming up Hoot…</h1></section>
  {:else if phaseIs('selection')}
    <section class="stage selection-stage">
      <div class="stage-heading">
        <div><p class="eyebrow">Choose a quiz</p><h1>What are we playing?</h1></div>
        <button class="secondary" on:click={() => send({ type: 'host_reload_content' })}>↻ Reload games file</button>
      </div>
      <div class="game-grid">
        {#each state.games as game}
          <button class="game-card" on:click={() => send({ type: 'host_select_game', game_id: game.id })}>
            <span class="game-count">{game.questionCount} questions</span>
            <strong>{game.title}</strong>
            <span>{game.description || 'Ready when you are.'}</span>
          </button>
        {/each}
      </div>
    </section>
  {:else if phaseIs('lobby')}
    <section class="lobby stage">
      <div class="lobby-copy">
        <p class="eyebrow">Players, join now</p>
        <h1>{selectedGame()?.title}</h1>
        <div class="join-url">{state.joinUrl}</div>
        {#if state.networkWarning}
          <div class="network-warning" role="alert">
            <strong>⚠ This address may not be reachable from phones.</strong>
            <p>{state.networkWarning}</p>
          </div>
        {/if}
        {#if state.joinUrls.length > 1}
          <label for="join-url">Network address</label>
          <select id="join-url" value={state.joinUrl} on:change={(event) => send({ type: 'host_set_join_url', url: event.currentTarget.value })}>
            {#each state.joinUrls as url}<option value={url}>{url}</option>{/each}
          </select>
        {/if}
        <div class="lobby-actions">
          <button class="primary huge" disabled={!state.players.length} on:click={() => send({ type: 'host_start' })}>Start Hoot!</button>
          <button class="secondary" on:click={() => send({ type: 'host_reload_content' })}>Reload content</button>
        </div>
      </div>
      <div class="qr-card">
        {#if qrCode}<img src={qrCode} alt={`QR code for ${state.joinUrl}`} />{/if}
        <strong>Scan to join</strong>
      </div>
      <div class="roster-panel">
        <h2>{state.players.length} {state.players.length === 1 ? 'player' : 'players'}</h2>
        <div class="name-cloud" aria-live="polite">
          {#each state.players as player}<span class:disconnected={!player.connected}>{player.username}</span>{/each}
        </div>
      </div>
    </section>
  {:else if phaseIs('reading', 'answering', 'reveal') && state.question}
    <section class="question-stage stage">
      {#if state.question.doublePoints}<div class="double-banner">×2 DOUBLE POINTS!</div>{/if}
      <div class="question-badge">Question {state.questionNumber} / {state.questionCount}</div>
      {#if deadline()}<Countdown deadlineMs={deadline()!} {serverOffsetMs} />{/if}
      <h1>{state.question.prompt}</h1>
      {#if state.question.imageUrl}<img class="question-image" src={state.question.imageUrl} alt={state.question.imageAlt || ''} />{/if}
      {#if !phaseIs('reading') && state.question.type === 'multiple_choice'}
        <div class="answer-grid">
          {#each state.question.options || [] as choice}
            <div class="answer-tile {choice.color}" class:correct-answer={phaseIs('reveal') && choice.correct} class:dimmed={phaseIs('reveal') && !choice.correct}>
              <span class="shape" aria-hidden="true">{answerSymbol(choice)}</span>
              <span>{choice.text}</span>
              {#if phaseIs('reveal') && state.distribution}
                <div class="answer-bar" style="width: {optionSharePercent(choice)}%"></div>
                <strong class="answer-count">{optionCount(choice)}</strong>
              {/if}
            </div>
          {/each}
        </div>
      {:else if phaseIs('reveal')}
        <div class="text-reveal">
          <p class="eyebrow">Accepted answer</p>
          <h2>{state.question.acceptedAnswers?.[0]}</h2>
          <div class="distribution-row">
            <span class="correct-pill">✓ {state.distribution?.correct || 0} correct</span>
            <span>× {state.distribution?.incorrect || 0} incorrect</span>
            <span>— {state.distribution?.unanswered || 0} unanswered</span>
          </div>
        </div>
      {/if}
      {#if phaseIs('reading')}
        <button class="primary floating-action" on:click={() => send({ type: 'host_advance' })}>Start answering now →</button>
      {:else if phaseIs('answering')}
        <button class="primary floating-action" on:click={() => send({ type: 'host_advance' })}>Reveal answer →</button>
      {:else if phaseIs('reveal')}
        <button class="primary floating-action" on:click={() => send({ type: 'host_advance' })}>
          {state.questionNumber === state.questionCount ? 'Show final leaderboard' : 'Show leaderboard'} →
        </button>
      {/if}
    </section>
  {:else if phaseIs('leaderboard')}
    <section class="stage leaderboard-stage">
      <p class="eyebrow">After question {state.questionNumber}</p><h1>Leaderboard</h1>
      <ol class="leaderboard">
        {#each state.players.slice(0, 5) as player}
          <li><span class="rank">{player.rank}</span><strong>{player.username}</strong><span class:rank-up={player.rankDelta > 0} class:rank-down={player.rankDelta < 0}>{rankMovement(player.rankDelta)}</span><b>{player.score.toLocaleString()}</b></li>
        {/each}
      </ol>
      <button class="primary floating-action" on:click={() => send({ type: 'host_advance' })}>Next question →</button>
    </section>
  {:else if phaseIs('final_leaderboard')}
    <section class="stage final-stage">
      <div class="stage-heading"><div><p class="eyebrow">That’s a wrap</p><h1>Final leaderboard</h1></div></div>
      <ol class="leaderboard final-list">
        {#each state.players as player}<li><span class="rank">{player.rank}</span><strong>{player.username}</strong><b>{player.score.toLocaleString()}</b></li>{/each}
      </ol>
      <h2>Play another</h2>
      <div class="game-grid compact">
        {#each state.games as game}<button class="game-card" on:click={() => send({ type: 'host_select_game', game_id: game.id })}><strong>{game.title}</strong><span>{game.questionCount} questions</span></button>{/each}
      </div>
    </section>
  {/if}
</main>
