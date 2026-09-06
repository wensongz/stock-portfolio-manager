// @ts-nocheck
import test from 'node:test';
import assert from 'node:assert/strict';
import { transactionBatchData, holdingBatchData, batchPreviewRequest } from './batchAdapters.ts';
import { parseThsCsv } from '../../pages/Transactions/thsCsvParser.ts';

test('preview preserves original metadata and edited normalized values for selected rows', () => {
  const row = { key:'3', selected:true, raw:'original,line', external_id:'trade-1', symbol:' AAPL ', stock_name:'Apple', transaction_type:'BUY', traded_at:'2026-01-01T00:00:00Z', shares:2, price:4, total_amount:8, commission:1 };
  const request = batchPreviewRequest({requestId:'retry-id',accountId:'a',source:'broker',kind:'transactions',fileName:'x.csv',sourceContent:'full file',rows:[row,{...row,key:'4',selected:false}],toData:r=>transactionBatchData(r,'US')});
  assert.equal(request.request_id,'retry-id');
  assert.equal(request.source_content,'full file');
  assert.equal(request.rows.length,1);
  assert.deepEqual(request.rows[0],{key:'3',raw:'original,line',external_id:'trade-1',data:{symbol:'AAPL',name:'Apple',market:'US',currency:'USD',transaction_type:'BUY',shares:2,price:4,total_amount:8,commission:1,traded_at:'2026-01-01T00:00:00.000Z',notes:null}});
});
test('holding payload preserves supplied market currency category and cost', () => {
 assert.deepEqual(holdingBatchData({accountId:'a',symbol:'HKD',name:'Cash',market:'HK',currency:'HKD',shares:20,avgCost:1,categoryId:'cash'}),{symbol:'HKD',name:'Cash',market:'HK',currency:'HKD',shares:20,avg_cost:1,category_id:'cash'});
});
test('THS retains raw rows and distinct execution ids even for equal trades', () => {
 const header='成交日期,证券代码,证券名称,操作,成交价格,成交数量,成交金额,成交编号';
 const a='20260101,600000,浦发银行,买入,10,100,1000,001';
 const b='20260101,600000,浦发银行,买入,10,100,1000,002';
 const rows=parseThsCsv([header,a,b].join('\n'));
 assert.equal(rows.length,2); assert.equal(rows[0].raw,a); assert.equal(rows[0].external_id,'001'); assert.equal(rows[1].external_id,'002');
});

test('placeholder execution ids do not collapse distinct dividends', () => {
 const rows=parseThsCsv('成交日期,证券代码,操作,成交数量,发生金额,成交编号\n20260101,600000,红利,0,20,0\n20260101,600001,红利,0,30,000');
 assert.equal(rows.length,2); assert.equal(rows[0].external_id,null); assert.equal(rows[1].external_id,null);
});

test('invalid row dates are retained for backend row validation without blocking other rows', () => {
 const base={key:'a',selected:true,symbol:'AAPL',stock_name:'Apple',transaction_type:'BUY',shares:1,price:10,total_amount:10,commission:0};
 const result=batchPreviewRequest({requestId:'id',accountId:'a',source:'broker',kind:'transactions',fileName:'a.csv',sourceContent:'original',rows:[{...base,traded_at:'invalid date'},{...base,key:'b',traded_at:'2026-01-01T00:00:00Z'}],toData:r=>transactionBatchData(r,'US')});
 assert.equal(result.rows[0].data.traded_at,'invalid date'); assert.equal(result.rows[1].data.traded_at,'2026-01-01T00:00:00.000Z');
});
