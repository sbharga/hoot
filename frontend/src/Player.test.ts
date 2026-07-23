import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import Player from './Player.svelte';

describe('player controller', () => {
  beforeEach(() => localStorage.clear());

  it('offers an accessible username-only join form', () => {
    render(Player);
    expect(screen.getByRole('heading', { name: 'Join the Hoot!' })).toBeInTheDocument();
    expect(screen.getByLabelText('Your name')).toHaveAttribute('maxlength', '24');
    expect(screen.getByRole('button', { name: 'Let’s go!' })).toBeEnabled();
    expect(screen.queryByLabelText(/pin/i)).not.toBeInTheDocument();
  });
});

