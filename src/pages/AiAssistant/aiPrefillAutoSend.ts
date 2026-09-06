import type { AiToolContext } from "../../types";
import type { ChatSendResult } from "../../stores/chatStore";
import type { AiPrefillRequest } from "./prefill";

interface AiPrefillStagingDependencies {
  stageSkill: (skill: string) => void;
  stageTool: (tool: AiToolContext) => void;
}

export type AiPrefillAutoSendPhase =
  | "READY"
  | "CREATING"
  | "CLAIMING"
  | "SENDING"
  | "CANCELLED"
  | "SUCCEEDED"
  | "FAILED";

export interface AiPrefillAutoSendOperation {
  readonly token: string;
  readonly selectionRevision: number;
  phase: AiPrefillAutoSendPhase;
  expectedSessionId: string | null;
}

let nextOperationId = 0;

export function createAiPrefillAutoSendOperation(
  selectionRevision: number,
): AiPrefillAutoSendOperation {
  nextOperationId += 1;
  return {
    token: `ai-prefill-${nextOperationId}`,
    selectionRevision,
    phase: "READY",
    expectedSessionId: null,
  };
}

interface OwnedContextCleanup {
  clearOwnedContext: (ownerToken: string) => void;
}

export function cancelAiPrefillAutoSendOperation(
  operation: AiPrefillAutoSendOperation,
  dependencies: OwnedContextCleanup,
): void {
  if (
    operation.phase === "CANCELLED" ||
    operation.phase === "SUCCEEDED" ||
    operation.phase === "FAILED" ||
    operation.phase === "SENDING"
  ) {
    return;
  }
  operation.phase = "CANCELLED";
  dependencies.clearOwnedContext(operation.token);
}

function operationOwnsPendingCreation(
  operation: AiPrefillAutoSendOperation | null,
): operation is AiPrefillAutoSendOperation {
  return Boolean(
    operation &&
      (operation.phase === "READY" ||
        operation.phase === "CREATING" ||
        operation.phase === "CLAIMING"),
  );
}

function operationOwnsExpectedSession(
  operation: AiPrefillAutoSendOperation | null,
  sessionId: string,
): boolean {
  return Boolean(
    operation &&
      operation.expectedSessionId === sessionId &&
      (operation.phase === "CLAIMING" || operation.phase === "SENDING"),
  );
}

function operationWasCancelled(operation: AiPrefillAutoSendOperation): boolean {
  return operation.phase === "CANCELLED";
}

export type AiSessionTransitionDecision =
  | "CANCEL_AUTO_AND_LOAD"
  | "CLEAR"
  | "KEEP_IN_FLIGHT"
  | "LOAD"
  | "UNCHANGED";

export function decideAiSessionTransition(input: {
  nextSessionId: string | null;
  loadedSessionId: string | null;
  expectedCreatedSessionId: string | null;
  autoSendOperation: AiPrefillAutoSendOperation | null;
}): AiSessionTransitionDecision {
  if (input.nextSessionId === null) return "CLEAR";
  if (operationOwnsExpectedSession(input.autoSendOperation, input.nextSessionId)) {
    return "KEEP_IN_FLIGHT";
  }
  if (operationOwnsPendingCreation(input.autoSendOperation)) {
    return "CANCEL_AUTO_AND_LOAD";
  }
  if (input.loadedSessionId === input.nextSessionId) return "UNCHANGED";
  return input.expectedCreatedSessionId === input.nextSessionId
    ? "KEEP_IN_FLIGHT"
    : "LOAD";
}

export function stageNonAutoAiPrefill(
  request: AiPrefillRequest | null,
  dependencies: AiPrefillStagingDependencies,
): string | null {
  if (!request || request.autoSend) return null;
  if (request.activeSkill) dependencies.stageSkill(request.activeSkill);
  if (request.toolContext) {
    dependencies.stageTool({
      name: request.toolContext.name,
      arguments: { ...request.toolContext.arguments },
    });
  }
  return request.prompt;
}

export function shouldAutoSendPrefill(input: {
  request: AiPrefillRequest | null;
  consumed: boolean;
  configured: boolean;
  sending: boolean;
}): boolean {
  return Boolean(
    input.request?.autoSend &&
      !input.consumed &&
      input.configured &&
      !input.sending,
  );
}

interface SelectionState {
  revision: number;
  currentSessionId: string | null;
}

export interface AiPrefillAutoSendDependencies extends OwnedContextCleanup {
  stageOwnedContext: (
    ownerToken: string,
    skill: string,
    tool: AiToolContext,
  ) => void;
  ownsStagedContext: (
    ownerToken: string,
    skill: string,
    tool: AiToolContext,
  ) => boolean;
  createSession: () => Promise<string>;
  getSelectionState: () => SelectionState;
  claimSession: (expectedRevision: number, sessionId: string) => boolean;
  sendMessage: (prompt: string, sessionId: string) => Promise<ChatSendResult>;
  touchSession: (sessionId: string) => Promise<void>;
  renameSession: (sessionId: string, prompt: string) => Promise<void>;
}

export type AiPrefillAutoSendResult =
  | { status: "cancelled" }
  | { status: "sent"; sessionId: string };

function exactTrustedRebalanceContext(request: AiPrefillRequest): AiToolContext {
  const args = request.toolContext?.arguments;
  const keys = args && typeof args === "object" ? Object.keys(args) : [];
  if (
    !request.autoSend ||
    request.prompt.trim().length === 0 ||
    request.activeSkill !== "portfolio-rebalance" ||
    request.toolContext?.name !== "get_rebalance_context" ||
    keys.length !== 1 ||
    keys[0] !== "config_id" ||
    typeof args?.config_id !== "string" ||
    args.config_id.trim().length === 0
  ) {
    throw new Error("自动发送请求缺少可信技能或工具上下文");
  }
  return {
    name: "get_rebalance_context",
    arguments: { config_id: args.config_id },
  };
}

export async function runAiPrefillAutoSend(
  request: AiPrefillRequest,
  operation: AiPrefillAutoSendOperation,
  dependencies: AiPrefillAutoSendDependencies,
): Promise<AiPrefillAutoSendResult> {
  const toolContext = exactTrustedRebalanceContext(request);
  if (operation.phase === "CANCELLED") return { status: "cancelled" };

  operation.phase = "CREATING";
  dependencies.stageOwnedContext(
    operation.token,
    "portfolio-rebalance",
    toolContext,
  );

  try {
    const createdSessionId = await dependencies.createSession();
    if (createdSessionId.trim().length === 0) {
      throw new Error("创建会话返回了无效的会话 ID");
    }
    if (operationWasCancelled(operation)) return { status: "cancelled" };

    const selection = dependencies.getSelectionState();
    const stillOwnsRequest = dependencies.ownsStagedContext(
      operation.token,
      "portfolio-rebalance",
      toolContext,
    );
    if (
      selection.revision !== operation.selectionRevision ||
      selection.currentSessionId !== null ||
      !stillOwnsRequest
    ) {
      cancelAiPrefillAutoSendOperation(operation, dependencies);
      return { status: "cancelled" };
    }

    operation.expectedSessionId = createdSessionId;
    operation.phase = "CLAIMING";
    if (!dependencies.claimSession(operation.selectionRevision, createdSessionId)) {
      cancelAiPrefillAutoSendOperation(operation, dependencies);
      return { status: "cancelled" };
    }

    const claimedSelection = dependencies.getSelectionState();
    if (
      claimedSelection.currentSessionId !== createdSessionId ||
      !dependencies.ownsStagedContext(
        operation.token,
        "portfolio-rebalance",
        toolContext,
      )
    ) {
      cancelAiPrefillAutoSendOperation(operation, dependencies);
      return { status: "cancelled" };
    }

    operation.phase = "SENDING";
    const sendResult = await dependencies.sendMessage(request.prompt, createdSessionId);
    if (!sendResult.ok) throw new Error(sendResult.error);

    await dependencies.touchSession(createdSessionId);
    await dependencies.renameSession(createdSessionId, request.prompt);
    operation.phase = "SUCCEEDED";
    return { status: "sent", sessionId: createdSessionId };
  } catch (error) {
    if (!operationWasCancelled(operation)) operation.phase = "FAILED";
    dependencies.clearOwnedContext(operation.token);
    throw error;
  }
}
