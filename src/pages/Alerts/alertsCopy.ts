export const ALERTS_MENU_LABEL = "投资提醒";

export const INVESTMENT_ALERT_TAB_KEYS = {
  portfolio: "portfolio",
  price: "price",
} as const;

export const DEFAULT_INVESTMENT_ALERT_TAB =
  INVESTMENT_ALERT_TAB_KEYS.portfolio;

export const INVESTMENT_ALERT_TABS = [
  { key: INVESTMENT_ALERT_TAB_KEYS.portfolio, label: "组合提醒" },
  { key: INVESTMENT_ALERT_TAB_KEYS.price, label: "价格提醒" },
] as const;
