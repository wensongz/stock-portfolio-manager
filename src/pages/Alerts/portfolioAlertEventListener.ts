import type { PortfolioAlertNotification } from "../../types";

type PortfolioAlertEventListener = (
  eventName: string,
  handler: (event: { payload: PortfolioAlertNotification }) => void,
) => Promise<() => void>;

export function startPortfolioAlertEventListener(
  listenForEvent: PortfolioAlertEventListener,
  ingestNotification: (notification: PortfolioAlertNotification) => void,
): Promise<() => void> {
  return listenForEvent("portfolio-alert-triggered", (event) => {
    ingestNotification(event.payload);
  });
}
