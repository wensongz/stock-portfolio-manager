import type { ReactNode } from "react";
import {
  DEFAULT_INVESTMENT_ALERT_TAB,
  INVESTMENT_ALERT_TABS,
  INVESTMENT_ALERT_TAB_KEYS,
} from "./alertsCopy.ts";

type AlertTabContent = {
  label: string;
  children: ReactNode;
};

export interface InvestmentAlertsTabItemsInput {
  portfolioTab: AlertTabContent;
  priceTab: AlertTabContent;
}

export function buildInvestmentAlertsTabs({
  portfolioTab,
  priceTab,
}: InvestmentAlertsTabItemsInput) {
  const tabContentByKey = {
    [INVESTMENT_ALERT_TAB_KEYS.portfolio]: portfolioTab,
    [INVESTMENT_ALERT_TAB_KEYS.price]: priceTab,
  } as const;

  return {
    defaultActiveKey: DEFAULT_INVESTMENT_ALERT_TAB,
    items: INVESTMENT_ALERT_TABS.map(({ key }) => ({
      key,
      label: tabContentByKey[key].label,
      children: tabContentByKey[key].children,
    })),
  };
}
