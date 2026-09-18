"use client";

/*
 * Hoodit branding layer for the embedded Aomi widget.
 *
 * `@aomi-labs/widget-lib` hard-codes its own mark, wordmark, composer
 * placeholder, and welcome suggestions. Visual swaps live in app.css; this
 * file handles the parts CSS cannot reach: placeholder text, the welcome
 * title, and a Hoodit-specific set of suggested actions that send through
 * the widget's own composer.
 */

import { useEffect, useState, type RefObject } from "react";
import { createPortal } from "react-dom";

const WELCOME_TITLE = "What should we explore on Robinhood Chain?";
const WELCOME_PLACEHOLDER = "Tell Hoodit what to trade…";
const REPLY_PLACEHOLDER = "Reply to Hoodit…";

const SUGGESTIONS = [
  {
    label: "Buy $100 of NVDA",
    prompt: "Buy $100 of NVDA stock token on Robinhood Chain",
  },
  {
    label: "Research TSLA on-chain",
    prompt: "Show the on-chain market and liquidity for the canonical TSLA stock token on Robinhood Chain",
  },
  {
    label: "Show my Robinhood Chain balances",
    prompt:
      "Show my wallet balances on Robinhood Chain without fetching valuation quotes",
  },
  {
    label: "Sell half my AAPL",
    prompt: "Sell half of my AAPL stock token position on Robinhood Chain",
  },
  {
    label: "Find active pools",
    prompt: "Show the most active pools on Robinhood Chain and explain the liquidity and recent volume",
  },
];

function patchText(root: HTMLElement) {
  root
    .querySelectorAll<HTMLTextAreaElement>("textarea.aui-composer-input")
    .forEach((input) => {
      const want = input.closest(".aui-thread-welcome-root")
        ? WELCOME_PLACEHOLDER
        : REPLY_PLACEHOLDER;
      if (input.placeholder !== want) input.placeholder = want;
    });
  root
    .querySelectorAll<HTMLElement>(".aui-thread-welcome-title")
    .forEach((title) => {
      if (title.textContent !== WELCOME_TITLE)
        title.textContent = WELCOME_TITLE;
    });
  root
    .querySelectorAll<HTMLButtonElement>(
      'button[aria-label="Switch Aomi product"]',
    )
    .forEach((button) => {
      button.setAttribute("aria-label", "Hoodit");
      button.tabIndex = -1;
    });
}

/** Fill the widget's composer through React's own value setter, then send. */
function sendPrompt(root: HTMLElement, prompt: string) {
  const input = root.querySelector<HTMLTextAreaElement>(
    "textarea.aui-composer-input",
  );
  if (!input) return;
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )?.set;
  setValue?.call(input, prompt);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  requestAnimationFrame(() => {
    const send = root.querySelector<HTMLButtonElement>(
      "button.aui-composer-send",
    );
    if (send && !send.disabled) send.click();
    else input.focus();
  });
}

function sameList(a: HTMLElement[], b: HTMLElement[]) {
  return a.length === b.length && a.every((el, i) => el === b[i]);
}

/** Static chip standing in for the widget's network selector, which hides itself when only one chain is configured. */
function ChainLock() {
  return (
    <span
      className="hoodit-chain-lock"
      title="Hoodit trades on Robinhood Chain only"
    >
      <i aria-hidden="true" />
      Robinhood Chain
      <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
        <path
          d="M4 7V5a4 4 0 1 1 8 0v2h1v7H3V7h1Zm2 0h4V5a2 2 0 1 0-4 0v2Z"
          fill="currentColor"
        />
      </svg>
    </span>
  );
}

export function HooditBrand({
  frameRef,
}: {
  frameRef: RefObject<HTMLElement | null>;
}) {
  const [suggestionHost, setSuggestionHost] = useState<HTMLElement | null>(
    null,
  );
  const [controlHosts, setControlHosts] = useState<HTMLElement[]>([]);

  useEffect(() => {
    const root = frameRef.current;
    if (!root) return;
    const sync = () => {
      patchText(root);
      const host = root.querySelector<HTMLElement>(
        ".aui-thread-welcome-suggestions",
      );
      setSuggestionHost((current) => (current === host ? current : host));
      const bars = [
        ...root.querySelectorAll<HTMLElement>(".aui-composer-action-scroll"),
      ];
      setControlHosts((current) => (sameList(current, bars) ? current : bars));
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["placeholder"],
    });
    return () => observer.disconnect();
  }, [frameRef]);

  const chips = controlHosts.map((host, i) =>
    createPortal(<ChainLock key={i} />, host),
  );
  if (!suggestionHost) return <>{chips}</>;
  return (
    <>
      {chips}
      {createPortal(
        <div
          className="hoodit-suggestions"
          role="group"
          aria-label="Suggested trades"
        >
          {SUGGESTIONS.map((item) => (
            <button
              key={item.label}
              type="button"
              className="hoodit-suggestion"
              aria-label={item.prompt}
              onClick={() =>
                frameRef.current && sendPrompt(frameRef.current, item.prompt)
              }
            >
              <i aria-hidden="true" />
              {item.label}
            </button>
          ))}
        </div>,
        suggestionHost,
      )}
    </>
  );
}
