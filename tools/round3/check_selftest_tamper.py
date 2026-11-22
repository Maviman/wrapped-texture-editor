import sys,os
from PIL import Image
d=sys.argv[1]
f=[x for x in os.listdir(d) if '_L01_' in x][0]
im=Image.open(os.path.join(d,f)).convert('RGBA'); a=im.load()
w,h=im.size
done=False
for y in range(h):
    for x in range(w):
        if a[x,y][3]>0 and not done:
            r,g,b,al=a[x,y]; a[x,y]=((r+1)%256,g,b,al); done=True
im.save(os.path.join(d,f))
p=os.path.join(d,'sphere.psd'); b=open(p,'rb').read(); open(p,'wb').write(b[:-1])
