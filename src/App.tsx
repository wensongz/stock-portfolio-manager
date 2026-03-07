import { BrowserRouter, Routes, Route, Link, Navigate } from 'react-router-dom';
import { Layout, Menu } from 'antd';
import {
  BankOutlined,
  TagsOutlined,
  StockOutlined,
  SwapOutlined,
} from '@ant-design/icons';
import AccountsPage from './pages/AccountsPage';
import CategoriesPage from './pages/CategoriesPage';
import HoldingsPage from './pages/HoldingsPage';
import TransactionsPage from './pages/TransactionsPage';

const { Header, Content, Sider } = Layout;

function App() {
  return (
    <BrowserRouter>
      <Layout style={{ minHeight: '100vh' }}>
        <Header
          style={{
            display: 'flex',
            alignItems: 'center',
            padding: '0 24px',
          }}
        >
          <h1 style={{ color: '#fff', margin: 0, fontSize: 18 }}>
            📊 Stock Portfolio Manager
          </h1>
        </Header>
        <Layout>
          <Sider width={200} style={{ background: '#fff' }}>
            <Menu
              mode="inline"
              defaultSelectedKeys={['accounts']}
              style={{ height: '100%', borderRight: 0 }}
              items={[
                {
                  key: 'accounts',
                  icon: <BankOutlined />,
                  label: <Link to="/accounts">Accounts</Link>,
                },
                {
                  key: 'categories',
                  icon: <TagsOutlined />,
                  label: <Link to="/categories">Categories</Link>,
                },
                {
                  key: 'holdings',
                  icon: <StockOutlined />,
                  label: <Link to="/holdings">Holdings</Link>,
                },
                {
                  key: 'transactions',
                  icon: <SwapOutlined />,
                  label: <Link to="/transactions">Transactions</Link>,
                },
              ]}
            />
          </Sider>
          <Layout style={{ padding: '24px' }}>
            <Content>
              <Routes>
                <Route path="/accounts" element={<AccountsPage />} />
                <Route path="/categories" element={<CategoriesPage />} />
                <Route path="/holdings" element={<HoldingsPage />} />
                <Route path="/transactions" element={<TransactionsPage />} />
                <Route path="*" element={<Navigate to="/accounts" replace />} />
              </Routes>
            </Content>
          </Layout>
        </Layout>
      </Layout>
    </BrowserRouter>
  );
}

export default App;
