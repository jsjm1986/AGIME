f='output/gen_ppt.py'
t=open(f,encoding='utf-8').read()
t=t.replace('align="center"','align=PP_ALIGN.CENTER')
open(f,'w',encoding='utf-8').write(t)
print('ok',t.count('PP_ALIGN.CENTER'))
