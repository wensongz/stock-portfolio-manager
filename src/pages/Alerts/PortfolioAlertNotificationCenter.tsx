import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { notification } from "antd";
import type { PortfolioAlertNotification } from "../../types";
import {
  portfolioAlertScopeKey,
  portfolioAlertStore,
  usePortfolioAlertStore,
} from "../../stores/portfolioAlertStore";
import { buildPortfolioAlertNotificationPresentation } from "./portfolioAlertViewModel";
import { startPortfolioAlertEventListener } from "./portfolioAlertEventListener";

export default function PortfolioAlertNotificationCenter() {
  const pendingNotifications = usePortfolioAlertStore(
    (state) => state.pendingNotifications,
  );
  const takePendingNotifications = usePortfolioAlertStore(
    (state) => state.takePendingNotifications,
  );
  const [notificationApi, notificationHolder] = notification.useNotification();

  useEffect(() => {
    if (pendingNotifications.length === 0) return;
    for (const breach of takePendingNotifications()) {
      const presentation = buildPortfolioAlertNotificationPresentation(breach);
      notificationApi.warning({
        message: presentation.title,
        description: presentation.description,
        placement: "topRight",
      });
    }
  }, [notificationApi, pendingNotifications, takePendingNotifications]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void startPortfolioAlertEventListener(
      (eventName, handler) => listen<PortfolioAlertNotification>(eventName, handler),
      (incoming) => {
        const store = portfolioAlertStore.getState();
        store.ingestNotification(incoming);
        if (portfolioAlertScopeKey(incoming.scope) === store.selectedScopeKey) {
          void store.loadScope(incoming.scope);
        }
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return notificationHolder;
}
