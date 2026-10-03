"use client";

/*
 * Widget configuration for Hoodit's web chat. Loaded with `ssr: false` from
 * hoodit-app.tsx so wallet SDKs never run on the server. The Privy provider
 * import registers Privy quick sign-in next to browser wallets.
 */

import { AomiWidget, robinhood, type CrossOriginWidgetAuth } from "@aomi-labs/widget-lib";
import "@aomi-labs/widget-lib/providers/privy";
import { aomiApiUrl, hooditAppId } from "../../lib/aomi-target";

// Every turn goes straight to the Hoodit app instead of Aomi's auto router,
// which otherwise picks generic apps (e.g. DefiLlama) for Hoodit questions.
const routing = { targets: [{ mode: "direct", apps: [{ applicationId: Number(hooditAppId), app: "hoodit" }] }] } as const;
const privyAppId = process.env.NEXT_PUBLIC_PRIVY_APP_ID?.trim();

/** Privy owns sign-in when an app id is configured; otherwise plain browser wallets. */
const auth: CrossOriginWidgetAuth = privyAppId
  ? { kind: "embedded_wallet", provider: "privy", appId: privyAppId }
  : { kind: "browser_wallet" };

export default function HooditWidget() {
  return (
    <AomiWidget
      applicationId={hooditAppId}
      apiUrl={aomiApiUrl}
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
      // Hoodit runs on the backend's default model; no picker.
      controlBarProps={{ hideModel: true }}
      routing={routing}
      // Hoodit pins its own cream palette in app.css, so the widget's
      // light/dark toggle would have no visible effect. The capability
      // library would offer other Aomi apps inside a Hoodit-only chat.
      features={{ theme: false, library: false }}
      // Guest credentials are page-scoped: never revive a conversation
      // owned by a previous anonymous identity after a reload.
      persistThread={false}
    />
  );
}
