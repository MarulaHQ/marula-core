// Admin contact — the locked screen's "Contact your admin" leg.
//
// Purpose: hand the tech to their own mail app with the ask pre-written;
// no in-app messaging, no pretend send.

import { Linking } from "react-native";

export async function openAdminMail(): Promise<void> {
  const subject = encodeURIComponent("Approve my phone for Marula");
  const body = encodeURIComponent(
    "Hi,\n\nMy phone is waiting on admin approval for Marula. Could you approve it so I can sign in?\n\nThanks,",
  );
  try {
    await Linking.openURL(`mailto:?subject=${subject}&body=${body}`);
  } catch {
    // No mail app on the device — silent; the screen remains the truth.
  }
}
