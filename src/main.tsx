import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { App as AntdApp, ConfigProvider } from "antd";
import zhCN from "antd/locale/zh_CN";
import App from "./App";
import "./styles/global.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <BrowserRouter>
      <ConfigProvider locale={zhCN}>
        <AntdApp>
          <App />
        </AntdApp>
      </ConfigProvider>
    </BrowserRouter>
  </React.StrictMode>
);
