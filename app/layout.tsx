import type { Metadata } from "next";
import "@fontsource/archivo-black/400.css";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";
import "@fontsource/work-sans/400.css";
import "@fontsource/work-sans/500.css";
import "@fontsource/work-sans/600.css";
import "@fontsource/work-sans/700.css";
import "@fontsource/work-sans/800.css";
import "./globals.css";

const productionHost = process.env.VERCEL_PROJECT_PRODUCTION_URL;
const siteUrl = productionHost
  ? `https://${productionHost}`
  : "http://localhost:3000";

const title = "Hoodit — Talk it. Trade it.";
const shareDescription = "AI trading on Robinhood Chain.";

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title,
  description: "Research tokens, read your wallet, and trade on Robinhood Chain with an AI agent.",
  openGraph: {
    title,
    description: shareDescription,
    type: "website",
    images: [{ url: "/og.png", width: 1200, height: 630, alt: title }],
  },
  twitter: { card: "summary_large_image", title, description: shareDescription, images: ["/og.png"] },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
