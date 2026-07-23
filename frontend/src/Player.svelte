<script lang="ts">
  import { onMount } from 'svelte';
  import Countdown from './lib/Countdown.svelte';
  import { connect, type ConnectionStatus } from './lib/connection';
  import type { Choice, PlayerState } from './lib/types';

  let state: PlayerState | undefined;
  let username = '';
  let textAnswer = '';
  let error = '';
  let joining = false;
  let status: ConnectionStatus = 'disconnected';
  let connection: ReturnType<typeof connect> | undefined;
  let serverOffsetMs = 0;
  let serverOffsetSet = false;

  onMount(() => {
    const token = localStorage.getItem('hoot.playerToken');
    if (token) openConnection(token);
    return () => connection?.close();
  });

  async function join() {
    error = '';
    joining = true;
    try {
      const response = await fetch('/api/players/join', {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ username })
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error || 'Could not join this Hoot.');
      localStorage.setItem('hoot.playerToken', body.token);
      openConnection(body.token);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Could not join this Hoot.';
    } finally { joining = false; }
  }

  function openConnection(token: string) {
    connection?.close();
    serverOffsetSet = false;
    connection = connect('player', token, (next: PlayerState) => {
      if (state?.phase.name !== next.phase.name) textAnswer = '';
      state = next;
      username = next.self.username;
      if (!serverOffsetSet) {
        serverOffsetMs = next.serverTimeMs - Date.now();
        serverOffsetSet = true;
      }
      error = '';
    }, (message, authenticationFailed) => {
      error = message;
      if (authenticationFailed) {
        localStorage.removeItem('hoot.playerToken');
        state = undefined;
        connection?.close();
      }
    }, next => status = next);
  }

  function choose(choice: Choice) { connection?.send({ type: 'submit_multiple_choice', option_id: choice.id }); }
  function submitText() {
    if (textAnswer.trim()) connection?.send({ type: 'submit_free_text', text: textAnswer });
  }
  function symbol(choice: Choice) { return choice.shape === 'triangle' ? '▲' : choice.shape === 'diamond' ? '◆' : choice.shape === 'circle' ? '●' : '■'; }
  function deadline() { return state && 'deadline_ms' in state.phase ? state.phase.deadline_ms : undefined; }
</script>

<svelte:head><title>Play Hoot!</title></svelte:head>

<main class="player-shell">
  {#if status !== 'connected' && state}<div class="connection-banner" aria-live="polite">Reconnecting… your place is saved.</div>{/if}
  {#if error}<div class="toast error" role="alert">{error}</div>{/if}
  {#if state && state.questionNumber && ['reading', 'answering', 'reveal'].includes(state.phase.name)}
    <div class="question-badge">Question {state.questionNumber} / {state.questionCount}</div>
  {/if}

  {#if !state}
    <section class="join-card">
      <p class="eyebrow">Ready to play?</p><h1>Join the Hoot!</h1>
      <form on:submit|preventDefault={join}>
        <label for="username">Your name</label>
        <input id="username" bind:value={username} maxlength="24" autocomplete="nickname" placeholder="Type your name" required />
        <button class="primary huge" type="submit" disabled={joining}>{joining ? 'Joining…' : 'Let’s go!'}</button>
      </form>
    </section>
  {:else}
    <section class="controller" aria-live="polite">
      {#if state.phase.name === 'selection'}
        <div class="controller-message"><h1>Great game!</h1><p>Hang tight while the host picks the next one.</p></div>
      {:else if state.phase.name === 'lobby'}
        <div class="controller-message"><span class="success-check" aria-hidden="true">✓</span><h1>You’re in, {state.self.username}!</h1><p>{state.playerCount} {state.playerCount === 1 ? 'player is' : 'players are'} ready.</p><small>Watch the shared screen.</small></div>
      {:else if state.phase.name === 'reading'}
        <div class="controller-message"><h1>Eyes up!</h1><p>Read the question on the shared screen.</p>{#if deadline()}<Countdown deadlineMs={deadline()!} {serverOffsetMs} />{/if}</div>
      {:else if state.phase.name === 'answering' && !state.eligible}
        <div class="controller-message"><h1>Almost there</h1><p>You joined during this question. You’ll play the next one.</p></div>
      {:else if state.phase.name === 'answering' && state.submitted}
        <div class="controller-message"><h1>Answer locked!</h1><p>Look at the shared screen for the result.</p>{#if deadline()}<Countdown deadlineMs={deadline()!} {serverOffsetMs} />{/if}</div>
      {:else if state.phase.name === 'answering' && state.controls?.kind === 'multiple_choice'}
        <div class="controller-top"><h1>Choose your answer</h1>{#if deadline()}<Countdown deadlineMs={deadline()!} {serverOffsetMs} />{/if}</div>
        <div class="controller-grid">
          {#each state.controls.options as choice}
            <button class="controller-button {choice.color}" on:click={() => choose(choice)} aria-label={`Answer ${choice.number}, ${choice.color} ${choice.shape}`}>
              <span aria-hidden="true">{symbol(choice)}</span><b>{choice.number}</b>
            </button>
          {/each}
        </div>
      {:else if state.phase.name === 'answering' && state.controls?.kind === 'free_text'}
        <div class="controller-top"><h1>Type your answer</h1>{#if deadline()}<Countdown deadlineMs={deadline()!} {serverOffsetMs} />{/if}</div>
        <form class="text-answer-form" on:submit|preventDefault={submitText}>
          <label for="answer">Your answer</label>
          <input id="answer" bind:value={textAnswer} maxlength="120" autocomplete="off" required />
          <button class="primary huge" type="submit">Lock it in</button>
        </form>
      {:else if state.phase.name === 'reveal' && state.result}
        <div class:result-correct={state.result.correct} class:result-wrong={!state.result.correct} class="result-card">
          <span class="result-icon" aria-hidden="true">{state.result.correct ? '✓' : '×'}</span>
          <h1>{state.result.correct ? 'Nailed it!' : 'Not this time'}</h1>
          <strong class="points">+{state.result.points.toLocaleString()}</strong>
          {#if state.result.answer?.kind === 'free_text'}<p>You said: {state.result.answer.submitted}</p>{/if}
          <p>Total: {state.self.score.toLocaleString()} points · Rank {state.self.rank}</p>
        </div>
      {:else if state.phase.name === 'reveal'}
        <div class="controller-message"><h1>Time’s up</h1><p>Check the shared screen for the answer.</p></div>
      {:else if state.phase.name === 'leaderboard'}
        <div class="controller-message">
          <p class="eyebrow">Current place</p><h1>#{state.self.rank}</h1><p>{state.self.score.toLocaleString()} points</p>
          {#if state.self.rankDelta > 0}<strong class="rank-up">↑ Up {state.self.rankDelta}!</strong>{/if}
          {#if state.ahead}
            <p>You’re {state.ahead.gap.toLocaleString()} {state.ahead.gap === 1 ? 'point' : 'points'} behind {state.ahead.username}!</p>
          {:else if state.self.rank === 1}
            <p>You’re in the lead!</p>
          {/if}
        </div>
      {:else if state.phase.name === 'final_leaderboard'}
        <div class="controller-message"><p class="eyebrow">Final result</p><h1>#{state.self.rank}</h1><p>{state.self.score.toLocaleString()} points</p><small>Stay here—the host can start another Hoot.</small></div>
      {/if}
    </section>
  {/if}

  {#if state}
    <footer class="player-footer">
      <span>{state.self.score.toLocaleString()} pts</span>
      <span>{state.self.username}</span>
    </footer>
  {/if}
</main>
