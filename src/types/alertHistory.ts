export interface AlertHistoryDetail {
  label: string;
  value: string;
}

export interface AlertHistoryMessage {
  id: string;
  kind: "PRICE" | "PORTFOLIO";
  title: string;
  message: string;
  scopeName: string;
  accountName: string | null;
  triggeredAt: string;
  details: AlertHistoryDetail[];
}

export interface AlertHistoryQuery {
  kind?: "PRICE" | "PORTFOLIO";
  search?: string;
  page?: number;
  pageSize?: number;
}

export interface AlertHistoryPage {
  items: AlertHistoryMessage[];
  total: number;
  page: number;
  pageSize: number;
}
