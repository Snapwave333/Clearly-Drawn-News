#!/usr/bin/env python3
"""Authorized, one-video-at-a-time YouTube Analytics API retention collector.
Reference utility. Set YOUTUBE_OAUTH_ACCESS_TOKEN to a legitimate, currently valid YouTube OAuth access token.
It does not perform OAuth consent/refresh or upload videos, and never logs access tokens.
"""
import argparse,json,os,sys,urllib.parse,urllib.request,urllib.error
from datetime import date,datetime,timezone
from pathlib import Path
ENDPOINT='https://youtubeanalytics.googleapis.com/v2/reports'

def call(params, token):
    url=ENDPOINT+'?'+urllib.parse.urlencode(params,quote_via=urllib.parse.quote)
    req=urllib.request.Request(url,headers={'Authorization':'Bearer '+token,'Accept':'application/json','User-Agent':'ATLAS-STUDIO/1.0'})
    try:
        with urllib.request.urlopen(req,timeout=30) as r:return json.loads(r.read().decode('utf-8'))
    except urllib.error.HTTPError as exc:
        # Do not print response body, which may contain personal or auth details.
        raise RuntimeError('YouTube Analytics HTTP '+str(exc.code)+'; check OAuth scopes, ownership, data maturity and compatible metrics') from None
    except urllib.error.URLError as exc:
        raise RuntimeError('Network error reaching YouTube Analytics; request was not completed') from None

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--video-id',required=True,help='Exactly one channel-owned video ID')
    ap.add_argument('--start-date',required=True,help='YYYY-MM-DD')
    ap.add_argument('--end-date',required=True,help='YYYY-MM-DD')
    ap.add_argument('--out',required=True)
    ap.add_argument('--dry-run',action='store_true',help='Show parameter names and scope requirements only; no network request')
    args=ap.parse_args()
    for x in [args.start_date,args.end_date]:date.fromisoformat(x)
    if args.end_date<args.start_date:raise SystemExit('end-date precedes start-date')
    if ',' in args.video_id or not args.video_id.strip():raise SystemExit('Only one video ID per call')
    params={'ids':'channel==MINE','startDate':args.start_date,'endDate':args.end_date,'dimensions':'elapsedVideoTimeRatio','filters':'video=='+args.video_id,'metrics':'audienceWatchRatio,relativeRetentionPerformance'}
    if args.dry_run:
        print(json.dumps({'api':ENDPOINT,'query':params,'requires':['https://www.googleapis.com/auth/yt-analytics.readonly','https://www.googleapis.com/auth/youtube.readonly'],'note':'token handled by caller, never printed'},indent=2));return
    token=os.environ.get('YOUTUBE_OAUTH_ACCESS_TOKEN')
    if not token:raise SystemExit('YOUTUBE_OAUTH_ACCESS_TOKEN missing; authorize the owner via YouTube OAuth first')
    try:raw=call(params,token)
    except RuntimeError as exc:raise SystemExit(str(exc))
    # Preserve official headers/rows, never invent absent data. Response empty rows are legitimate.
    payload={'video_id':args.video_id,'source':'youtube_analytics_reports.query','retrieved_at':datetime.now(timezone.utc).isoformat(),'start_date':args.start_date,'end_date':args.end_date,'data_status':'available' if raw.get('rows') else 'not_available','columnHeaders':raw.get('columnHeaders',[]),'rows':raw.get('rows',[])}
    out=Path(args.out);out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text(json.dumps(payload,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print('Wrote report:',out,'rows:',len(payload['rows']),'status:',payload['data_status'])
if __name__=='__main__':main()
