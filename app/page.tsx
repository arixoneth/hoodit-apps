import type { Metadata } from "next";
import { HooditLanding } from "./hoodit-landing";

export const metadata: Metadata = {
  title: "Hoodit — Talk it. Trade it.",
  description:
    "Research tokens, read your wallet, and trade on Robinhood Chain from Telegram or the web.",
};

export default function Home() {
  return <HooditLanding />;
}
