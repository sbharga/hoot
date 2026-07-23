export type Phase =
  | { name: 'selection' }
  | { name: 'lobby'; game_id: string }
  | { name: 'reading'; question_index: number; deadline_ms: number }
  | { name: 'answering'; question_index: number; started_at_ms: number; deadline_ms: number }
  | { name: 'reveal'; question_index: number }
  | { name: 'leaderboard'; question_index: number }
  | { name: 'final_leaderboard' };

export interface GameSummary { id: string; title: string; description: string; questionCount: number }
export interface RankedPlayer { id: string; username: string; score: number; rank: number; rankDelta: number; connected?: boolean }
export interface Choice { id: string; text: string; number: number; shape: string; color: string; correct?: boolean }
export interface Question {
  type: 'multiple_choice' | 'free_text';
  id: string;
  prompt: string;
  imageUrl?: string;
  imageAlt?: string;
  timeLimitSeconds: number;
  readingTimeSeconds: number;
  doublePoints: boolean;
  options?: Choice[];
  acceptedAnswers?: string[];
}

export interface HostState {
  role: 'host'; revision: number; serverTimeMs: number; phase: Phase;
  games: GameSummary[]; players: RankedPlayer[]; question?: Question;
  questionNumber?: number; questionCount?: number; distribution?: any;
  joinUrls: string[]; joinUrl?: string; networkWarning?: string;
}

export interface PlayerState {
  role: 'player'; revision: number; serverTimeMs: number; phase: Phase;
  self: RankedPlayer; playerCount: number; eligible: boolean; submitted: boolean;
  controls?: { kind: 'multiple_choice'; options: Choice[] } | { kind: 'free_text' };
  result?: { correct: boolean; points: number; answer: any };
  questionNumber?: number; questionCount?: number;
  ahead?: { username: string; gap: number };
}

