export function isComposerPrimaryActionDisabled(input: {
  pending: boolean;
  sending: boolean;
  canSend: boolean;
}): boolean {
  if (input.sending) return false;
  return input.pending || !input.canSend;
}
