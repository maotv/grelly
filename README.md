# Grelly - The Git Release Tool

This aims to be a replacement for git-describe which 
will generate Semantic Versions for git projects.
The major version will come from either the branch name
or a version tag or version commit (release: 17.0.0) 
The minor version will come from a version tag or commit 
and grelly (--release) will be able to generate such 
commits and tags.
The patch version will be teh number of commits from
the last release version, similar to git-describe
grelly --release will also be able to maintain a 
changelog file.