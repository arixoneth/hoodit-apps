"use client";

/*
 * Hoodit welcome screen inside the Aomi widget: title and suggested trades.
 * AomiWidget has no props for either yet (placeholders and logos are CSS in
 * app.css), so this patches the welcome root. Delete it once widget-lib
 * forwards welcomeTitle and suggestions.
 */

import { useEffect, useState, type RefObject } from "react";
import { createPortal } from "react-dom";

const WELCOME_TITLE = "What should we ape today?";

const SUGGESTIONS = [
  { label: "Buy $100 of NVDA", prompt: "Buy $100 of NVDA stock token on Robinhood Chain" },
  {
    label: "Research TSLA on-chain",
    prompt: "Show the on-chain market and liquidity for the canonical TSLA stock token on Robinhood Chain",
  },
  {
    label: "Show my Robinhood Chain balances",
    prompt: "Show my wallet balances on Robinhood Chain without fetching valuation quotes",
  },
  { label: "Sell half my AAPL", prompt: "Sell half of my AAPL stock token position on Robinhood Chain" },
  {
    label: "Find active pools",
    prompt: "Show the most active pools on Robinhood Chain and explain the liquidity and recent volume",
  },
];

/** Type into the widget's composer the way a user would, then submit its form. */
function sendPrompt(welcome: HTMLElement, prompt: string) {
  const input = welcome.querySelector<HTMLElement>('.aui-composer-input [role="textbox"]');
  if (!input) return;
  input.textContent = prompt;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  requestAnimationFrame(() => input.closest("form")?.requestSubmit());
}

export function HooditBrand({ frameRef }: { frameRef: RefObject<HTMLElement | null> }) {
  const [welcome, setWelcome] = useState<HTMLElement | null>(null);

  useEffect(() => {
    const frame = frameRef.current;
    if (!frame) return;
    const sync = () => {
      const root = frame.querySelector<HTMLElement>(".aui-thread-welcome-root");
      const title = root?.querySelector(".aui-thread-welcome-title");
      if (title && title.textContent !== WELCOME_TITLE) title.textContent = WELCOME_TITLE;
      setWelcome(root);
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(frame, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [frameRef]);

  const host = welcome?.querySelector(".aui-thread-welcome-suggestions");
  if (!welcome || !host) return null;
  return createPortal(
    <div className="hoodit-suggestions" role="group" aria-label="Suggested trades">
      {SUGGESTIONS.map((item) => (
        <button
          key={item.label}
          type="button"
          className="hoodit-suggestion"
          aria-label={item.prompt}
          onClick={() => sendPrompt(welcome, item.prompt)}
        >
          <i aria-hidden="true" />
          {item.label}
        </button>
      ))}
    </div>,
    host,
  );
}
