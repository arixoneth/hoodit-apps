/*
 * Hand-drawn riso-style illustrations for Hoodit.
 * Palette lives in globals.css; the SVGs read it through CSS vars so they retheme
 * with the page. Standalone copies for marketing live in brand-kit/illustrations.
 */

type SvgProps = { className?: string; title?: string };

const ink = "var(--ink)";
const cobalt = "var(--cobalt)";
const green = "var(--green)";
const mint = "var(--mint)";
const mustard = "var(--mustard)";

/** Pile of bills with a coin. Fund step. */
export function MoneyScene({ className, title = "A pile of bills" }: SvgProps) {
  const bill = (x: number, y: number, r: number, key: string) => (
    <g key={key} transform={`translate(${x} ${y}) rotate(${r})`} strokeLinecap="round" strokeLinejoin="round">
      <rect x="-46" y="-26" width="92" height="52" rx="3" fill={mint} stroke={ink} strokeWidth="4" />
      <rect x="-38" y="-18" width="76" height="36" rx="2" fill="none" stroke={green} strokeWidth="2" />
      <path d="M-30 -8 C-22 -14 -14 -2 -6 -8 S10 -14 18 -8 M-30 2 C-20 -4 -10 8 0 2 S16 -4 26 2 M-30 12 C-22 6 -12 18 -2 12 S12 6 22 12" stroke={green} strokeWidth="2.5" fill="none" />
      <circle cx="0" cy="0" r="9" fill={mint} stroke={green} strokeWidth="2.5" />
      <path d="M0 -6 L0 6 M-3 -3 Q0 -6 3 -3 Q0 0 -3 3 Q0 6 3 3" stroke={green} strokeWidth="2" fill="none" />
    </g>
  );
  return (
    <svg className={className} viewBox="0 0 260 170" role="img" aria-label={title}>
      {bill(70, 118, -14, "a")}
      {bill(190, 112, 9, "b")}
      {bill(126, 82, -4, "c")}
      {bill(116, 52, 6, "d")}
      <g transform="translate(222 46)" strokeLinejoin="round">
        <circle r="22" fill={mustard} stroke={ink} strokeWidth="4" />
        <path d="M0 -13 L0 13 M-7 -6 Q0 -12 7 -6 Q0 0 -7 6 Q0 12 7 6" stroke={ink} strokeWidth="3.5" fill="none" strokeLinecap="round" />
      </g>
      <g transform="translate(30 42)" strokeLinejoin="round">
        <circle r="15" fill={mustard} stroke={ink} strokeWidth="3.5" />
        <path d="M0 -9 L0 9 M-5 -4 Q0 -8 5 -4 Q0 0 -5 4 Q0 8 5 4" stroke={ink} strokeWidth="3" fill="none" strokeLinecap="round" />
      </g>
    </svg>
  );
}

/** Bar columns with an up arrow and coins. The edge section. */
export function ChartScene({ className, title = "A rising chart" }: SvgProps) {
  const col = (x: number, h: number, key: string) => (
    <g key={key} strokeLinejoin="round">
      <path d={`M${x} ${190 - h} L${x + 12} ${182 - h} L${x + 42} ${182 - h} L${x + 42} 182 L${x + 30} 190 L${x} 190Z`} fill={cobalt} stroke={ink} strokeWidth="4" />
      <path d={`M${x + 30} 190 L${x + 30} ${190 - h} L${x} ${190 - h}`} fill="none" stroke={ink} strokeWidth="4" />
      <rect x={x + 6} y={196 - h} width="18" height={h - 6} fill={green} />
      <path d={`M${x + 30} ${190 - h} L${x + 42} ${182 - h}`} stroke={ink} strokeWidth="4" />
    </g>
  );
  return (
    <svg className={className} viewBox="0 0 300 200" role="img" aria-label={title}>
      {col(24, 60, "1")}
      {col(84, 92, "2")}
      {col(144, 76, "3")}
      {col(204, 130, "4")}
      <path d="M18 132 L70 92 L104 116 L176 46" fill="none" stroke={green} strokeWidth="14" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M150 34 L194 28 L188 72Z" fill={green} stroke={green} strokeWidth="6" strokeLinejoin="round" />
      <g transform="translate(262 40)" strokeLinejoin="round">
        <circle r="18" fill={mustard} stroke={ink} strokeWidth="3.5" />
        <path d="M0 -11 L0 11 M-6 -5 Q0 -10 6 -5 Q0 0 -6 5 Q0 10 6 5" stroke={ink} strokeWidth="3" fill="none" strokeLinecap="round" />
      </g>
      <g transform="translate(232 108) rotate(-18)" strokeLinejoin="round">
        <ellipse rx="16" ry="10" fill={mustard} stroke={ink} strokeWidth="3" />
      </g>
      <g transform="translate(118 20) rotate(14)" strokeLinejoin="round">
        <ellipse rx="14" ry="9" fill={mustard} stroke={ink} strokeWidth="3" />
      </g>
    </svg>
  );
}

