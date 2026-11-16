// Command palette and chat panel — the public surface of this module.
//
// Mount `CommandPaletteHost` once in the app chrome (AppHeader). Pages that
// want a richer context chip publish their view, row count and selection with
// `useScreenContextRegistration`; nothing else needs to change.

export { CommandPalette, type CommandPaletteProps } from "./command-palette";
export { CommandPaletteHost } from "./command-palette-host";
export { ChatPanel } from "./chat-panel";
export { sendChatTurn } from "./chat-turn";
export { GENERAL_CHAT_SURFACE, useGeneralChatGrant, type GrantStatus } from "./grant";
export {
  contextPrompt,
  describeScreenContext,
  screenLabelFor,
  setScreenContext,
  useScreenContext,
  useScreenContextRegistration,
  type ScreenContext,
} from "./screen-context";
export { actions, chatSessionId, getSnapshot, subscribe, type ChatMessage, type PaletteState } from "./store";
