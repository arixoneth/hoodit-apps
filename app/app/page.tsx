import type { Metadata } from "next";
import { HooditApp } from "./hoodit-app";

export const metadata: Metadata = {
  title: "Hoodit — App",
  description: "Research tokens, read your wallet, and trade on Robinhood Chain with Hoodit.",
};

export default function AppPage() {
  return <HooditApp />;
}
