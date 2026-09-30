"use client";

import Image from "next/image";
import { useState } from "react";
import { ExecutionFixture } from "./execution-fixture";
import { ChartScene, MoneyScene } from "./illustrations";

const qrPattern = [
  "111111101011101111111",
  "100000101010101000001",
  "101110101110101011101",
  "101110100101101011101",
  "101110101011101011101",
  "100000100101001000001",
  "111111101010101111111",
  "000000001101100000000",
  "101111111001011101101",
  "011001001111100010110",
  "110101111010111110011",
  "001110001101000101100",
  "101011111011111011101",
  "000000001100100010010",
  "111111101011101110111",
  "100000101100100010101",
  "101110101011111110111",
  "101110100110100010100",
  "101110101101101111111",
  "100000101010000100100",
  "111111101101111111101",
];

export function HooditLanding() {
  const [qrFront, setQrFront] = useState(false);
  const toggleStage = () => setQrFront((value) => !value);
  const stageKey = (event: React.KeyboardEvent) => {
    if (event.key === "Enter" || event.key === " ") { event.preventDefault(); toggleStage(); }
  };
  return (
    <main className="site-shell">
      <div className="hero-zone">
      <header className="nav-shell">
        <a className="brand" href="#top" aria-label="Hoodit home">
          <Image className="brand-mark" src="/hoodit-logo.jpg" alt="" width={512} height={512} />
          <span>HOODIT</span>
        </a>
        <nav className="nav-links" aria-label="Sections">
          <a href="#demo">Demo</a>
          <a href="#edge">The edge</a>
          <a href="#features">Features</a>
          <a href="#how">How it works</a>
          <a href="#join">Join</a>
        </nav>
        <a className="telegram-link" href="#hero-qr">
          <span aria-hidden="true">↗</span> Telegram
        </a>
      </header>

      <section className="hero" id="top">
        <div className="hero-copy">
          <div className="eyebrow"><span className="eyebrow-live"><i /> LIVE ON</span><strong>ROBINHOOD CHAIN</strong></div>
          <h1>Talk it.<br /><em>Trade it.</em></h1>
          <p className="hero-dek">
            Research tokens, inspect your wallet, and turn plain English into
            reviewable on-chain trades. No command syntax. Just text Hoodit.
          </p>
          <div className="hero-actions">
            <button className="primary-button" type="button" aria-pressed={qrFront} onClick={() => setQrFront(true)}>Scan to enter <span>↗</span></button>
            <a className="text-link" href="#demo">Try it first <span>↓</span></a>
          </div>
          <div className="hero-proof">
            <div><strong>24/7</strong><span>AI desk</span></div>
            <div><strong>AOMI</strong><span>Agent runtime</span></div>
            <div><strong>ROBINHOOD</strong><span>Chain native</span></div>
          </div>
        </div>

        <div className="hero-poster-wrap" id="hero-qr">
          <div className={`hero-stage${qrFront ? " qr-front" : ""}`}>
            <div className="stage-item stage-video" role="button" tabIndex={0} aria-label={qrFront ? "Bring the bot back to the front" : "Show the QR code"} onClick={toggleStage} onKeyDown={stageKey}>
              <div className="hero-poster plate">
                <video className="poster-cat" src="/hero-cat.mp4" poster="/hero-cat.jpg" autoPlay muted loop playsInline aria-label="Hoodit, a cat wearing a hood" />
                <span className="poster-tag">HOODIT IN THE HOOD</span>
              </div>
            </div>
            <div className="stage-item stage-qr" role="button" tabIndex={0} aria-label={qrFront ? "Bring the bot back to the front" : "Show the QR code"} onClick={toggleStage} onKeyDown={stageKey}>
              <div className="qr-sticker">
                <div className="qr-code is-soon" role="img" aria-label="Hoodit Telegram bot, coming soon">
                  {qrPattern.join("").split("").map((cell, index) => <i key={index} className={cell === "1" ? "filled" : ""} />)}
                  <span><Image src="/hoodit-logo.jpg" alt="" width={512} height={512} /></span>
                  <b className="qr-soon">COMING SOON</b>
                </div>
                <div className="qr-caption"><span>SOON ON</span><b>TELEGRAM</b></div>
              </div>
            </div>
          </div>
          <div className="hero-qr-note"><i /> PRIVATE ACCESS · INSTANT SETUP</div>
        </div>
      </section>

      <div className="tape-x" aria-hidden="true">
        <div className="tape tape-a"><div>TALK IT <span>✦</span> TRADE IT <span>✦</span> NO COMMAND SYNTAX <span>✦</span> BUILT FOR ROBINHOOD CHAIN <span>✦</span> TALK IT <span>✦</span> TRADE IT <span>✦</span> NO COMMAND SYNTAX <span>✦</span> BUILT FOR ROBINHOOD CHAIN <span>✦</span></div></div>
        <div className="tape tape-b"><div>SCAN <span>✦</span> FUND <span>✦</span> TALK <span>✦</span> CONFIRM <span>✦</span> THE BOT IN THE HOOD <span>✦</span> SCAN <span>✦</span> FUND <span>✦</span> TALK <span>✦</span> CONFIRM <span>✦</span> THE BOT IN THE HOOD <span>✦</span></div></div>
      </div>

      <section className="widget-demo-section section-shell" id="demo">
        <div className="widget-demo-copy">
          <h2>Try the bot before you scan.</h2>
          <p>Same agent. Same transaction flow. Right here on the web.</p>
          <div className="demo-scene-frame">
            <video className="demo-scene" src="/demo-cat.mp4" poster="/demo-cat.jpg" autoPlay muted loop playsInline aria-label="Hoodit in a ninja stance" />
          </div>
        </div>
        <div className="widget-wrap">
          <div className="widget-kicker"><a className="primary-button demo-open" href="/app">Open app <span>↗</span></a></div>
          <ExecutionFixture />
        </div>
      </section>
      </div>

      <div className="tape" aria-hidden="true">
        <div>TEXT TO TRADE <span>✦</span> EXPLORE MARKETS <span>✦</span> READ YOUR WALLET <span>✦</span> REVIEW EXECUTION <span>✦</span> TEXT TO TRADE <span>✦</span> EXPLORE MARKETS <span>✦</span></div>
      </div>

      <section className="thesis section-shell" id="edge">
        <div className="section-number">01 / THE EDGE</div>
        <div className="thesis-copy">
          <p className="overline">YOUR NEW TRADING DESK</p>
          <h2>Less menu.<br /><em>More signal.</em></h2>
          <p>Hoodit is what happens when a Telegram-native trading bot gets a real reasoning layer. Describe the outcome. Hoodit builds the transaction. You stay in control.</p>
        </div>
        <ChartScene className="thesis-scene" />
        <div className="desk-readout plate">
          <div className="desk-status"><span><i /> ILLUSTRATIVE MARKET PREVIEW</span><b>EXAMPLE DATA</b></div>
          <div className="desk-intent">
            <small>YOUR INTENT</small>
            <p>“Show active pools with strong liquidity, then explain the tradeoffs.”</p>
            <span>VOICE / TEXT / TELEGRAM</span>
          </div>
          <div className="desk-resolve">
            <div><small>01 / READ</small><strong>Market + chain</strong><span>Public pool activity and wallet balances in context.</span></div>
            <div><small>02 / REASON</small><strong>Inspect the route</strong><span>Liquidity, activity, and quote limits made clear.</span></div>
            <div><small>03 / BUILD</small><strong>One reviewable tx</strong><span>Nothing moves until you confirm.</span></div>
          </div>
          <div className="desk-ticker" aria-hidden="true">
            <span>NVDA <b>+2.84%</b></span><i />
            <span>LIQUIDITY <b>$2.4M</b></span><i />
            <span>24H VOLUME <b>$840K</b></span><i />
            <span>EXECUTION <b>REVIEW REQUIRED</b></span>
          </div>
        </div>
      </section>

      <section className="features section-shell plate" id="features">
        <div className="feature-intro">
          <p className="overline">IN TELEGRAM · ON THE WEB</p>
          <h2>One chat.<br />A whole desk.</h2>
          <p>Access the same Hoodit intelligence wherever you trade—inside Telegram or through the Aomi widget.</p>
        </div>
        <div className="feature-grid">
          <article className="feature-card feature-salmon">
            <span className="feature-index">01</span>
            <div className="mini-stack"><i /><i /><i /></div>
            <h3>Discover markets,<br />with context.</h3>
            <p>Find active or newly indexed pools and inspect liquidity, volume, and recent trades.</p>
          </article>
          <article className="feature-card">
            <span className="feature-index">02</span>
            <div className="mini-chart"><i /><i /><i /><i /><i /></div>
            <h3>Wallet balances<br />without the noise.</h3>
            <p>Read current token balances and request bounded quote-based estimates when you need them.</p>
          </article>
          <article className="feature-card">
            <span className="feature-index">03</span>
            <div className="mini-prompt">“Keep risk under 2%.”<span>↵</span></div>
            <h3>Intent in.<br />Transaction out.</h3>
            <p>Hoodit turns natural-language strategy into a reviewable, confirm-before-execute transaction.</p>
          </article>
          <article className="feature-card feature-green">
            <span className="feature-index">04</span>
            <div className="model-badge">AI</div>
            <h3>An agent runtime<br />behind the chat.</h3>
            <p>Powered by Aomi for reasoning across public market data, wallet reads, and execution details.</p>
          </article>
        </div>
      </section>

      <section className="flow section-shell" id="how">
        <div className="flow-header">
          <p className="overline">ZERO COMMANDS TO MEMORIZE</p>
          <h2>From question to<br />reviewable trade.</h2>
        </div>
        <div className="flow-steps">
          <article>
            <b className="flow-number">1</b>
            <div className="flow-copy"><h3>Scan</h3><p>Open Hoodit from the QR code and connect your Telegram identity.</p><small className="flow-note">SESSION CREATED IN SECONDS</small></div>
            <div className="flow-visual scan-visual" aria-hidden="true"><i className="scan-ring scan-ring-outer" /><i className="scan-ring scan-ring-inner" /><Image className="scan-mark" src="/hoodit-logo.jpg" alt="" width={512} height={512} /><small className="scan-status">LINKED</small></div>
          </article>
          <article>
            <b className="flow-number">2</b>
            <div className="flow-copy"><h3>Fund</h3><p>Transfer money into your trading account on Robinhood Chain.</p><small className="flow-note">YOUR BALANCE · YOUR CONTROL</small></div>
            <div className="flow-visual balance-visual"><MoneyScene className="balance-scene" /><div><small className="balance-label">AVAILABLE TO TRADE</small><strong className="balance-amount">$1,240.00</strong><span className="balance-chain"><i /> USDC · ROBINHOOD CHAIN</span></div></div>
          </article>
          <article>
            <b className="flow-number">3</b>
            <div className="flow-copy"><h3>Talk</h3><p>Research a token or describe a trade. Review the transaction. Confirm when it looks right.</p><small className="flow-note">NO COMMAND SYNTAX</small></div>
            <div className="flow-visual talk-visual"><span className="talk-command">“Research this pool, then buy $25.”</span><i className="talk-arrow">→</i><b className="talk-review">REVIEW TX ↗</b></div>
          </article>
        </div>
      </section>

      <section className="join section-shell" id="join">
        <div className="join-card plate">
          <div className="join-copy">
            <p className="overline">EARLY ACCESS</p>
            <h2>Your next trade starts with a text.</h2>
            <p>Open Hoodit in Telegram. Your account, intelligence, and trading flow move with you.</p>
            <div className="join-actions">
              <a className="primary-button" href="#hero-qr">Open in Telegram <span>↗</span></a>
              <a className="join-link" href="/app">Or try the web app <span>↗</span></a>
            </div>
          </div>
          <div className="cta-poster">
            <Image className="cta-cat" src="/cta-cat.jpg" alt="Hoodit holding a sword, ready to trade" width={1000} height={1000} />
            <div className="closer-details">
              <div className="closer-row closer-account"><span><i /> @HOODIT_AI</span><b>ONLINE</b></div>
              <div className="closer-row"><span>IDENTITY</span><b>TELEGRAM</b></div>
              <div className="closer-row"><span>SETTLEMENT</span><b>ROBINHOOD CHAIN</b></div>
              <div className="closer-row"><span>FINAL SAY</span><b>ALWAYS YOURS</b></div>
            </div>
          </div>
        </div>
      </section>

      <footer className="footer section-shell">
        <div className="brand footer-brand"><Image className="brand-mark" src="/hoodit-logo.jpg" alt="" width={512} height={512} /><span>HOODIT</span></div>
        <p>Trade the idea. Keep the final say.</p>
        <p className="risk">Hoodit does not provide financial advice. Demo transactions are illustrative and require user review before execution.</p>
        <span>© 2026 HOODIT</span>
      </footer>
    </main>
  );
}
