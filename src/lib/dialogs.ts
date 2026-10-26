// Which title-bar dialog is open, so the command palette can open them too.
import { create } from "zustand";

export type DialogId = "usage" | "history" | "journal";

export const useDialogs = create<{ open: DialogId | null }>(() => ({ open: null }));
export const openDialog = (id: DialogId) => useDialogs.setState({ open: id });
export const closeDialog = () => useDialogs.setState({ open: null });
