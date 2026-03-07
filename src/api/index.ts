import { invoke } from '@tauri-apps/api/core';
import type {
  Account,
  CreateAccountRequest,
  UpdateAccountRequest,
  Category,
  CreateCategoryRequest,
  UpdateCategoryRequest,
  Holding,
  CreateHoldingRequest,
  UpdateHoldingRequest,
  Transaction,
  CreateTransactionRequest,
} from '../types';

// Account API
export const accountApi = {
  list: () => invoke<Account[]>('list_accounts'),
  get: (id: string) => invoke<Account>('get_account', { id }),
  create: (request: CreateAccountRequest) =>
    invoke<Account>('create_account', { request }),
  update: (id: string, request: UpdateAccountRequest) =>
    invoke<Account>('update_account', { id, request }),
  delete: (id: string) => invoke<void>('delete_account', { id }),
};

// Category API
export const categoryApi = {
  list: () => invoke<Category[]>('list_categories'),
  create: (request: CreateCategoryRequest) =>
    invoke<Category>('create_category', { request }),
  update: (id: string, request: UpdateCategoryRequest) =>
    invoke<Category>('update_category', { id, request }),
  delete: (id: string) => invoke<void>('delete_category', { id }),
};

// Holding API
export const holdingApi = {
  list: (accountId?: string) =>
    invoke<Holding[]>('list_holdings', { accountId: accountId ?? null }),
  get: (id: string) => invoke<Holding>('get_holding', { id }),
  create: (request: CreateHoldingRequest) =>
    invoke<Holding>('create_holding', { request }),
  update: (id: string, request: UpdateHoldingRequest) =>
    invoke<Holding>('update_holding', { id, request }),
  delete: (id: string) => invoke<void>('delete_holding', { id }),
};

// Transaction API
export const transactionApi = {
  list: (accountId?: string, symbol?: string) =>
    invoke<Transaction[]>('list_transactions', {
      accountId: accountId ?? null,
      symbol: symbol ?? null,
    }),
  create: (request: CreateTransactionRequest) =>
    invoke<Transaction>('create_transaction', { request }),
  delete: (id: string) => invoke<void>('delete_transaction', { id }),
};
