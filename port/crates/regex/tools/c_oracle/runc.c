/* Secondary oracle: TeX Live's bundled regex (texk/web2c/pdftexdir/regex)
   compiled with MSVC, driven like examples/runcase.rs.  Not part of the
   crate; see README.  Reads lines "cflags\teflags\tnmatch\tpat_hex\tsubj_hex"
   and prints "E:<regerror>", "0", or "1:so,eo;so,eo;..." per line. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <regex/regex.h>

static int unhex(const char *s, size_t n, char *out)
{
  size_t i;
  for (i = 0; i + 1 < n + 1 && i < n; i += 2)
    {
      unsigned v;
      if (sscanf(s + i, "%2x", &v) != 1)
        return -1;
      *out++ = (char) v;
    }
  *out = 0;
  return 0;
}

int main (void)
{
  static char line[1 << 20], pat[1 << 19], subj[1 << 19];
  setvbuf (stdout, NULL, _IONBF, 0);
  while (fgets (line, sizeof line, stdin))
    {
      int cflags, eflags, nmatch, ret, i;
      char *f[5], *p = line;
      regex_t re;
      regmatch_t *pm;
      size_t len = strlen (line);
      if (len && line[len - 1] == '\n') line[--len] = 0;
      if (len && line[len - 1] == '\r') line[--len] = 0;
      if (!len) continue;
      for (i = 0; i < 5; i++)
        {
          f[i] = p;
          p = strchr (p, '\t');
          if (p) *p++ = 0; else p = line + len;
        }
      cflags = atoi (f[0]); eflags = atoi (f[1]); nmatch = atoi (f[2]);
      unhex (f[3], strlen (f[3]), pat);
      unhex (f[4], strlen (f[4]), subj);
      ret = regcomp (&re, pat, cflags);
      if (ret)
        {
          char buf[200];
          regerror (ret, &re, buf, sizeof buf);
          printf ("E:%s\n", buf);
          continue;
        }
      pm = malloc (sizeof (regmatch_t) * (nmatch ? nmatch : 1));
      for (i = 0; i < nmatch; i++) pm[i].rm_so = pm[i].rm_eo = -7;
      ret = regexec (&re, subj, nmatch, pm, eflags);
      if (ret)
        printf ("0\n");
      else
        {
          printf ("1:");
          for (i = 0; i < nmatch; i++)
            printf ("%s%d,%d", i ? ";" : "", pm[i].rm_so, pm[i].rm_eo);
          printf ("\n");
        }
      free (pm);
      regfree (&re);
    }
  return 0;
}
