"use client";

/*
 * Widget configuration for Hoodit's web chat. Loaded with `ssr: false` from
 * hoodit-app.tsx so wallet SDKs never run on the server. The Privy provider
 * import registers Privy quick sign-in next to browser wallets.
 */

import { AomiWidget, robinhood, type CrossOriginWidgetAuth } from "@aomi-labs/widget-lib";
import "@aomi-labs/widget-lib/providers/privy";
import { chatFetch } from "../../lib/chat-fetch";

const clientOptions = { fetch: chatFetch };
const privyAppId = process.env.NEXT_PUBLIC_PRIVY_APP_ID?.trim();

/** Privy owns sign-in when an app id is configured; otherwise plain browser wallets. */
const auth: CrossOriginWidgetAuth = privyAppId
  ? { kind: "embedded_wallet", provider: "privy", appId: privyAppId }
  : { kind: "browser_wallet" };

export default function HooditWidget() {
  return (
    <AomiWidget
      applicationId="2938613"
      apiUrl="https://chat.aomi.dev"
      auth={auth}
      // Hoodit researches and trades tokens on Robinhood Chain only, so the network
      // selector and wallet connection are pinned to it.
      wallets={{
        evm: { preset: "popular", chains: [robinhood], appName: "Hoodit", appLogoUrl: "/hoodit-logo.jpg" },
        solana: false,
      }}
      walletFamilies={["evm"]}
      width="100%"
      height="100%"
      showHeader
      showSidebar
      walletPosition="footer"
      clientOptions={clientOptions}
      controlBarProps={{ hideApp: true }}
      // Guest credentials are page-scoped: never revive a conversation
      // owned by a previous anonymous identity after a reload.
      persistThread={false}
    />
  );
}
