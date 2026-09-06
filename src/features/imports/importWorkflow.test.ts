// @ts-nocheck -- Runs directly in Node without browser-focused Node typings.
import test from 'node:test';
import assert from 'node:assert/strict';
import { batchPreviewRequest, transactionBatchData } from './batchAdapters.ts';

test('OCR reimport identity remains original image and source row despite edits and new request', () => {
 const raw={stock_name:'Apple',traded_at:'2026-01-01T00:00:00Z',price:10,shares:1,total_amount:10,commission:0,transaction_type:'BUY'};
 const row={...raw,raw,key:'0',selected:true,symbol:'AAPL'};
 const preview=(requestId,rows)=>batchPreviewRequest({requestId,accountId:'account',source:'ths-ocr',kind:'transactions',fileName:'screen.png',sourceContent:'original-image-base64',rows,toData:r=>transactionBatchData(r,'US')});
 const first=preview('first',[row]); const repeat=preview('second',[{...row,stock_name:'Edited name',price:20}]);
 assert.notEqual(first.request_id,repeat.request_id);
 assert.equal(first.source_content,repeat.source_content);
 assert.equal(first.rows[0].key,repeat.rows[0].key);
 assert.deepEqual(first.rows[0].raw,repeat.rows[0].raw);
 assert.notDeepEqual(first.rows[0].data,repeat.rows[0].data);
});

test('staging filters only unchecked rows without dropping equal legitimate trades', () => {
 const row={key:'one',selected:true,raw:'first',symbol:'AAPL',stock_name:'Apple',traded_at:'2026-01-01T00:00:00Z',price:10,shares:1,total_amount:10,commission:0,transaction_type:'BUY'};
 const result=batchPreviewRequest({requestId:'id',accountId:'a',source:'broker',kind:'transactions',fileName:'file.csv',sourceContent:'original',rows:[row,{...row,key:'two',raw:'second'},{...row,key:'skip',selected:false}],toData:r=>transactionBatchData(r,'US')});
 assert.deepEqual(result.rows.map(r=>r.key),['one','two']); assert.deepEqual(result.rows[0].data,result.rows[1].data);
});
