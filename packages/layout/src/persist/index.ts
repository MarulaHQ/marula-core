/**
 * @module Persistence barrel: the store contract plus shipped adapters.
 *
 * Purpose: single import surface for {@link PanelStateStore},
 * {@link LocalStoragePanelStateStore}, and {@link MemoryPanelStateStore}.
 * Non-purpose: nothing else — engine consumers never need deeper paths.
 */

export { PanelPersistError, type PanelStateStore } from "./types";
export {
  LocalStoragePanelStateStore,
  StorageUnavailableError,
  type StorageLike,
  type LocalStorageStoreOptions,
} from "./local-storage";
export { MemoryPanelStateStore } from "./memory";
