"use client";

import { useEffect, useRef, useState } from "react";
import Image from "next/image";

type Step =
  | { kind: "tool"; title: string; chips: string[] }
  | { kind: "note"; text: string };

const prompt = "sell half my PONS into ETH. can i get out clean?";

const steps: Step[] = [
  {
    kind: "tool",
    title: "Activate skills",
    chips: ["hoodit/markets", "lifi_swap"],
  },
  {
    kind: "tool",
    title: "Read wallet holdings",
    chips: ["412,880 PONS", "0.31 ETH", "Robinhood Chain"],
  },
  {
    kind: "tool",
    title: "Check token and launch stage",
    chips: ["graduated", "no honeypot flags", "taxes unknown"],
  },
  {
    kind: "note",
    text: "Half is 206,440 PONS, sized exactly. Quoting that exact amount before staging anything.",
  },
  {
    kind: "tool",
    title: "Check exit at size",
    chips: ["route found", "loss 1.8%", "gas $0.02"],
  },
  {
    kind: "note",
    text: "The exit is clean at this size. Preparing the swap for your review.",
  },
  {
    kind: "tool",
    title: "Quote and stage swap",
    chips: ["206,440 PONS → ETH", "min out 0.0412 ETH"],
  },
  {
    kind: "tool",
    title: "Simulate transaction",
    chips: ["approval + swap", "simulation passed"],
  },
  {
    kind: "tool",
    title: "Send to wallet",
    chips: ["awaiting signature"],
  },
];

const TYPE_DELAY = 18;
const STEP_DELAY = 620;
const LOOP_DELAY = 3300;

export function ExecutionFixture() {
  const rootRef = useRef<HTMLDivElement>(null);
  const timers = useRef<number[]>([]);
  const typer = useRef<number | null>(null);
  const [run, setRun] = useState(0);
  const [started, setStarted] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [typed, setTyped] = useState(0);
  const [shown, setShown] = useState(0);
  const [approval, setApproval] = useState<"hidden" | "idle" | "pressed" | "signed">("hidden");
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [complete, setComplete] = useState(false);

  useEffect(() => {
    const node = rootRef.current;
    if (!node || started) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return;
        const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        setReducedMotion(reduceMotion);
        if (reduceMotion) {
          setTyped(prompt.length);
          setShown(steps.length);
          setApproval("signed");
          setCollapsed(true);
          setComplete(true);
        }
        setStarted(true);
        observer.disconnect();
      },
      { threshold: 0.35 },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [started]);

  useEffect(() => {
    if (!started || reducedMotion) return;

    typer.current = window.setInterval(() => {
      setTyped((count) => {
        if (count >= prompt.length) {
          if (typer.current !== null) window.clearInterval(typer.current);
          return count;
        }
        return count + 1;
      });
    }, TYPE_DELAY);

    const afterTyping = prompt.length * TYPE_DELAY + 420;
    steps.slice(0, -1).forEach((_, index) => {
      timers.current.push(
        window.setTimeout(() => setShown(index + 1), afterTyping + index * STEP_DELAY),
      );
    });
    const approveAt = afterTyping + (steps.length - 1) * STEP_DELAY + 180;
    timers.current.push(window.setTimeout(() => setApproval("idle"), approveAt));
    timers.current.push(window.setTimeout(() => setApproval("pressed"), approveAt + 900));
    timers.current.push(window.setTimeout(() => setApproval("signed"), approveAt + 1120));
    timers.current.push(window.setTimeout(() => setShown(steps.length), approveAt + 1620));
    timers.current.push(window.setTimeout(() => setCollapsed(true), approveAt + 2280));
    timers.current.push(window.setTimeout(() => setComplete(true), approveAt + 2680));
    timers.current.push(window.setTimeout(() => {
      setTyped(0);
      setShown(0);
      setApproval("hidden");
      setCollapsed(false);
      setExpanded(false);
      setComplete(false);
      setRun((value) => value + 1);
    }, approveAt + 2680 + LOOP_DELAY));

    return () => {
      timers.current.forEach(window.clearTimeout);
      timers.current = [];
      if (typer.current !== null) window.clearInterval(typer.current);
    };
  }, [reducedMotion, run, started]);

  const visible = steps.slice(0, shown);
  const toolCount = visible.filter((step) => step.kind === "tool").length;

  return (
    <div className="execution-fixture" ref={rootRef}>
      <div className="fixture-topbar">
        <div className="widget-agent">
          <Image className="agent-avatar" src="/hoodit-logo.jpg" alt="" width={512} height={512} />
          <div><strong>Hoodit</strong><small>AI trading agent</small></div>
        </div>
        <span className="live-pill"><i /> LIVE DEMO</span>
      </div>

      <div className="fixture-stage" aria-live="polite">
        {started && (
          <div className="fixture-user-bubble">
            {prompt.slice(0, typed)}
            {typed < prompt.length && <i className="type-cursor" />}
          </div>
        )}

        {!collapsed && typed >= prompt.length && (
          <div className="trace-card">
            <div className="trace-header">
              <span className="trace-working">Working</span>
              {toolCount > 0 && <span>{toolCount} {toolCount === 1 ? "step" : "steps"}</span>}
              <b>⌄</b>
            </div>
            {shown > 0 && (
              <div className="trace-viewport">
                {visible.map((step, index) => {
                  const active = index === visible.length - 1 && shown < steps.length;
                  if (step.kind === "note") {
                    return <div className="trace-note" key={`${run}-${index}`}><i />{step.text}</div>;
                  }
                  return (
                    <div className="trace-tool" key={`${run}-${index}`}>
                      <div className="trace-tool-title">
                        {active ? <i className="trace-spinner" /> : <i className="trace-check">✓</i>}
                        <span className={active ? "active" : ""}>{step.title}</span>
                      </div>
                      <div className="trace-chips">
                        {step.chips.map((chip) => <span key={chip}><i />{chip}</span>)}
                      </div>
                    </div>
                  );
                })}

                {approval !== "hidden" && (
                  <div className="approval-row">
                    <button className={`approval-button ${approval}`} type="button">
                      {approval === "signed" ? "✓ Signed" : "⌘ Approve & Sign"}
                    </button>
                    <span>keys never leave your wallet</span>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {collapsed && (
          <button className="worked-pill" type="button" onClick={() => setExpanded((value) => !value)} aria-expanded={expanded}>
            <span>✓</span><strong>Worked for 8s</strong><small>7 steps</small><b>{expanded ? "⌃" : "⌄"}</b>
          </button>
        )}

        {collapsed && expanded && (
          <div className="trace-card expanded-trace">
            <div className="trace-viewport">
              {steps.map((step, index) => step.kind === "note" ? (
                <div className="trace-note" key={index}><i />{step.text}</div>
              ) : (
                <div className="trace-tool" key={index}>
                  <div className="trace-tool-title"><i className="trace-check">✓</i><span>{step.title}</span></div>
                  <div className="trace-chips">{step.chips.map((chip) => <span key={chip}><i />{chip}</span>)}</div>
                </div>
              ))}
            </div>
          </div>
        )}

        {complete && (
          <div className="fixture-answer">
            <p>Sold 206,440 PONS for 0.0415 ETH. The receipt is confirmed on Robinhood Chain.</p>
            <div><span>tx 0xa73c…91e2</span><b>confirmed</b></div>
          </div>
        )}
      </div>

      <div className="widget-footer"><span>POWERED BY</span><b>AOMI</b><span className="chain-status"><i /> Robinhood Chain</span></div>
    </div>
  );
}
