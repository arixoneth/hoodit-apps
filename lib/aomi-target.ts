/** Aomi chat host and Hoodit application the web chat talks to. Defaults to
 * production; set both env vars to point a dev or preview build at staging.
 */
export const aomiApiUrl = process.env.NEXT_PUBLIC_AOMI_API_URL?.trim() || "https://chat.aomi.dev";
export const hooditAppId = process.env.NEXT_PUBLIC_HOODIT_APP_ID?.trim() || "2938613";
