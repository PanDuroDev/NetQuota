//! Blocked page: explains the rule, offers temporary access, never weakens the
//! stored quota. Temporary access is a timed rule removal handled by the worker.

export {};

const params = new URLSearchParams(location.search);
const host = params.get("host") ?? "";

function formatHost(value: string): string {
  return value.replace(/[<>&"']/g, "");
}

const title = document.getElementById("title")!;
const detail = document.getElementById("detail")!;
const temp = document.getElementById("temp") as HTMLButtonElement;
const open = document.getElementById("open") as HTMLButtonElement;

const safeHost = formatHost(host);
title.textContent = safeHost ? `${safeHost} is blocked` : "Site blocked";
detail.textContent =
  "The daily quota for this site was reached, or the site was blocked manually in NetQuota.";

temp.addEventListener("click", () => {
  if (!safeHost) return;
  void chrome.runtime
    .sendMessage({ type: "temp_access", host: safeHost })
    .then(() => {
      temp.disabled = true;
      temp.textContent = "Allowed for 5 minutes";
      window.setTimeout(() => {
        if (safeHost) location.replace(`https://${safeHost}/`);
      }, 800);
    })
    .catch(() => {
      temp.textContent = "Service unreachable — try again";
    });
});

open.addEventListener("click", () => {
  chrome.tabs.create({ url: chrome.runtime.getURL("popup.html") });
});
