// Entry redirect: send unanchored visits to Today (the default surface).
//
// Non-purpose: no rendering of its own.
import { redirect } from "next/navigation";

export default function Home() {
  redirect("/today");
}
